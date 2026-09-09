//! Read-only filesystem abstraction for loading game data files.
//!
//! All Rust-side persistence uses serde (JSON). SbFile only reads
//! binary game data (`.cpf` profiles, level files, sprite data, etc.).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use std::fs;

use robin_util::asset_fs::AssetBytes;

pub const SBFILE_NO_ERROR: i32 = 0;
pub const SBFILE_ERROR_FILE_NOT_FOUND: i32 = -1;
pub const SBFILE_ERROR_NO_FILE: i32 = -4;
pub const SBFILE_ERROR_READ: i32 = -5;
pub const SBFILE_ERROR_SEEK: i32 = -7;
pub const SBFILE_ERROR_PATH_ALREADY_PRESENT: i32 = -10;
pub const SBFILE_ERROR_PATH_NOT_IN_SET: i32 = -11;
pub const SBFILE_ERROR_BAD_ARCHIVE: i32 = -20;

pub const SB_FILE_READ: i32 = 0x01;

/// Instance-owned game-file lookup state.
///
/// The original game stored alternate paths in a static list and searched it
/// in insertion order after the requested path. This type preserves that
/// ordering while allowing independent instances for tools and tests.
pub struct SbFileSystem {
    /// Cache lineage survives immutable snapshots but not independently
    /// constructed authorities. It is runtime-only, never persisted authority.
    origin_identity: u64,
    /// Prepared native relative-path authority; legacy ingestion remains live.
    #[cfg(not(target_arch = "wasm32"))]
    working_directory: Option<PathBuf>,
    assets: Arc<robin_util::asset_fs::AssetVfs>,
    alternate_paths: Mutex<Vec<String>>,
    /// The selected locale root, fallback root, and presentation language.
    /// Keeping them behind
    /// one mutex makes a runtime language switch atomic: readers can observe
    /// either the old pair or the new pair, never a selected locale from one
    /// configuration and a fallback from another.
    locale_paths: Mutex<LocaleLookup>,
    overlay_paths: Mutex<Vec<OverlayRoot>>,
    primary_path: Mutex<Option<PathBuf>>,
    /// Irreversible one-job verifier confinement. When set, every legacy
    /// lookup bypasses overlays, locales, embedded VFS data, the process
    /// working directory, and alternate paths, and resolves only below this
    /// canonical root.
    ranked_verifier_primary_path: Mutex<Option<PathBuf>>,
    /// One-way closed lookup mode used only by the private official exporter.
    /// Direct host-CWD and unrooted alternate fallthrough are forbidden.
    official_projection_strict: AtomicBool,
}

/// Presentation language identity travels with its lookup roots, including in
/// prepared readers. A numeric/empty shipping root is not a language tag.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct LocaleLookup {
    selected: Option<String>,
    fallback: Option<String>,
    language: Option<String>,
}

/// Read-only proof of every process-global filesystem/VFS authority that can
/// affect game-data lookup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SbFileMountSnapshot {
    /// Pinned native CWD, or None for a live legacy/browser installation.
    pub working_directory: Option<PathBuf>,
    pub alternate_paths: Vec<String>,
    pub selected_locale: Option<String>,
    pub fallback_locale: Option<String>,
    pub overlay_paths: Vec<String>,
    pub primary_path: Option<PathBuf>,
    /// Confinement changes the lookup graph even if the primary root matches.
    pub ranked_verifier_primary_path: Option<PathBuf>,
    pub asset_vfs: robin_util::asset_fs::AssetVfsAuthoritySnapshot,
    pub official_projection_strict: bool,
}

impl SbFileSystem {
    /// The application-owned VFS used to configure this reader before mission
    /// snapshotting. Prepared readers must not be reconfigured after capture.
    pub fn asset_vfs(&self) -> &Arc<robin_util::asset_fs::AssetVfs> {
        &self.assets
    }

    /// Preserve configured mounts while attaching the VFS explicitly installed
    /// by this application. Startup performs this before exposing the reader.
    pub fn with_asset_vfs(&self, assets: Arc<robin_util::asset_fs::AssetVfs>) -> Self {
        let mut files = self.snapshot();
        files.origin_identity = next_reader_identity();
        files.assets = assets;
        files
    }

    /// Stable identity shared by this reader and its prepared snapshots.
    /// Cache keys must additionally include effective mounts and generations.
    pub fn origin_identity(&self) -> u64 {
        self.origin_identity
    }

    /// Selection of this reader's VFS, never the process-global installation.
    pub fn selection_snapshot(&self) -> robin_util::asset_fs::AssetSelection {
        self.assets.selection_snapshot()
    }

    pub fn new(assets: Arc<robin_util::asset_fs::AssetVfs>) -> Self {
        Self {
            origin_identity: next_reader_identity(),
            #[cfg(not(target_arch = "wasm32"))]
            working_directory: None,
            assets,
            alternate_paths: Mutex::new(Vec::new()),
            locale_paths: Mutex::new(LocaleLookup::default()),
            overlay_paths: Mutex::new(Vec::new()),
            primary_path: Mutex::new(None),
            ranked_verifier_primary_path: Mutex::new(None),
            official_projection_strict: AtomicBool::new(false),
        }
    }
}

fn next_reader_identity() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("file authority identity exhausted")
}

impl std::fmt::Debug for SbFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SbFileSystem")
            .field("authority", &self.mount_snapshot())
            .finish()
    }
}

static GLOBAL_FILE_SYSTEM: OnceLock<SbFileSystem> = OnceLock::new();

fn global_file_system() -> &'static SbFileSystem {
    GLOBAL_FILE_SYSTEM.get_or_init(|| SbFileSystem::new(robin_util::asset_fs::global().clone()))
}

/// One overlay root in the lookup stack.
///
/// `Directory` is a path on disk; lookups join it with the requested
/// path and consult the case-insensitive filesystem resolver.
/// `Zip` is a zip archive mounted in-memory (no extraction); lookups
/// consult a pre-built case-folded index built at mount time.
#[derive(Clone)]
enum OverlayRoot {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Directory(PathBuf),
    Zip(Arc<ZipOverlay>),
}

impl OverlayRoot {
    fn display_path(&self) -> std::borrow::Cow<'_, str> {
        match self {
            OverlayRoot::Directory(p) => p.to_string_lossy(),
            OverlayRoot::Zip(z) => std::borrow::Cow::Borrowed(z.display_path.as_str()),
        }
    }
}

/// A zip archive mounted as an overlay root.  Reads files on demand,
/// no on-disk extraction.
///
/// `index` maps **normalized + lowercased datadir paths** (e.g.
/// `data/levels/s02_lei_mp.rhm`) to a zip entry index.  The mapping
/// already accounts for the detected layout: a zip whose entries are
/// wrapped in an `English/` directory has that prefix stripped, and
/// a zip with bare `*.rhm` files at the root has `Data/Levels/`
/// prepended.  See `detect_zip_layout`.
struct ZipOverlay {
    display_path: String,
    archive: Mutex<ZipArchiveReader>,
    /// Lower-cased + slash-normalized datadir path → zip entry index.
    index: HashMap<String, usize>,
}

enum ZipArchiveReader {
    #[cfg(not(target_arch = "wasm32"))]
    File(zip::ZipArchive<fs::File>),
    Memory(zip::ZipArchive<Cursor<Arc<[u8]>>>),
}

impl ZipArchiveReader {
    fn entry_names(&mut self) -> Result<Vec<String>, i32> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::File(archive) => collect_zip_entry_names(archive),
            Self::Memory(archive) => collect_zip_entry_names(archive),
        }
    }

    fn read_entry(&mut self, index: usize) -> Result<Vec<u8>, String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::File(archive) => read_zip_entry(archive, index),
            Self::Memory(archive) => read_zip_entry(archive, index),
        }
    }
}

fn collect_zip_entry_names<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<String>, i32> {
    let mut entry_names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| SBFILE_ERROR_BAD_ARCHIVE)?;
        if entry.is_dir() {
            entry_names.push(String::new());
        } else {
            entry_names.push(entry.name().replace('\\', "/"));
        }
    }
    Ok(entry_names)
}

fn read_zip_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_index(index)
        .map_err(|error| format!("zip entry {index} open failed: {error}"))?;
    let capacity = usize::try_from(entry.size())
        .map_err(|_| format!("zip entry {index} is too large for this platform"))?;
    let mut bytes = Vec::with_capacity(capacity);
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format!("zip entry {index} read failed: {error}"))?;
    Ok(bytes)
}

impl ZipOverlay {
    /// Open a mission zip using the selected `.rhm` as the authoritative
    /// datadir root. This is required for archives containing several
    /// language roots and for older Spellforge archives wrapped in a
    /// mission-named directory.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_for_mission(path: &Path, selected_rhm_entry: Option<&str>) -> Result<Self, i32> {
        let file = fs::File::open(path).map_err(|e| {
            tracing::warn!("ZipOverlay::open: failed to open {}: {e}", path.display());
            SBFILE_ERROR_FILE_NOT_FOUND
        })?;
        let archive = zip::ZipArchive::new(file).map_err(|e| {
            tracing::warn!("ZipOverlay::open: not a valid zip {}: {e}", path.display());
            SBFILE_ERROR_BAD_ARCHIVE
        })?;

        Self::from_archive(
            path.to_string_lossy().into_owned(),
            ZipArchiveReader::File(archive),
            selected_rhm_entry,
        )
    }

    fn open_bytes_for_mission(
        display_path: String,
        bytes: Arc<[u8]>,
        selected_rhm_entry: Option<&str>,
    ) -> Result<Self, i32> {
        let archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
            tracing::warn!("ZipOverlay::open: not a valid in-memory zip {display_path}: {error}");
            SBFILE_ERROR_BAD_ARCHIVE
        })?;
        Self::from_archive(
            display_path,
            ZipArchiveReader::Memory(archive),
            selected_rhm_entry,
        )
    }

    fn from_archive(
        display_path: String,
        mut archive: ZipArchiveReader,
        selected_rhm_entry: Option<&str>,
    ) -> Result<Self, i32> {
        let entry_names = archive.entry_names()?;

        let (strip, prepend) = match selected_rhm_entry {
            Some(selected) => {
                detect_zip_layout_for_mission(&entry_names, selected).map_err(|e| {
                    tracing::warn!(
                        "ZipOverlay::open: selected mission layout is invalid in {}: {e}",
                        display_path
                    );
                    SBFILE_ERROR_BAD_ARCHIVE
                })?
            }
            None => detect_zip_layout(&entry_names),
        };
        tracing::info!(
            "ZipOverlay::open: {} (strip={:?}, prepend={:?}, entries={})",
            display_path,
            strip,
            prepend,
            entry_names.iter().filter(|n| !n.is_empty()).count()
        );

        let mut index = HashMap::new();
        for (i, name) in entry_names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            // Match strip prefix case-insensitively. Entries that don't
            // share the detected prefix are simply not indexed (they're
            // unreachable via the overlay namespace, which is fine —
            // they're typically things like screenshots inside the zip).
            let rest = if strip.is_empty() {
                name.as_str()
            } else if name.to_ascii_lowercase().starts_with(&strip) {
                &name[strip.len()..]
            } else {
                continue;
            };
            let mut key = String::with_capacity(prepend.len() + rest.len());
            key.push_str(&prepend);
            key.push_str(rest);
            let key = key.to_ascii_lowercase();
            // First entry wins on duplicate keys; zip should not have
            // duplicates but be defensive.
            index.entry(key).or_insert(i);
        }

        Ok(Self {
            display_path,
            archive: Mutex::new(archive),
            index,
        })
    }

    fn try_read(&self, path: &str) -> Option<Vec<u8>> {
        let key = path.replace('\\', "/").to_ascii_lowercase();
        let idx = *self.index.get(&key)?;
        let mut archive = self.archive.lock().unwrap();
        archive
            .read_entry(idx)
            .inspect_err(|error| tracing::warn!("ZipOverlay::try_read: {error}"))
            .ok()
    }

    fn exists(&self, path: &str) -> bool {
        let key = path.replace('\\', "/").to_ascii_lowercase();
        self.index.contains_key(&key)
    }
}

/// Detect the datadir layout inside a zip archive.
///
/// Returns `(strip_prefix, prepend_prefix)` — both lowercase, both end
/// with `/` when non-empty.  Entries are matched after lowercasing
/// against `strip_prefix`, and the remainder gets `prepend_prefix`
/// pasted in front to form the indexed key.
///
/// Layouts handled:
/// - `English/DATA/Levels/foo.rhm` → strip `english/`, prepend ``
/// - `English/2047/data/Text/Level.res` → strip `english/`, prepend ``
/// - `DATA/Levels/foo.rhm` → strip ``, prepend ``
/// - `2047/data/Text/Level.res` → strip ``, prepend ``
/// - `foo.rhm` (bare at root) → strip ``, prepend `data/levels/`
/// - `lib/api.lua` (Spellforge lib folder) → strip ``, prepend `data/levels/`
///
/// Public so the custom-mission picker can use the same logic to
/// filter out zip entries that would not be reachable through the
/// overlay (e.g. duplicate language-variant `.rhm` files: the
/// detector picks one locale folder and the others land outside the
/// indexed namespace).
pub fn detect_zip_layout(entries: &[String]) -> (String, String) {
    // First pass: find an entry whose path contains a "datadir root"
    // segment (either `Data/` or a numeric locale folder followed by
    // `data/`).  The bytes before that segment become the strip prefix.
    for entry in entries {
        if entry.is_empty() {
            continue;
        }
        let lower = entry.to_ascii_lowercase();
        let segments: Vec<&str> = lower.split('/').collect();
        for i in 0..segments.len() {
            // Numeric locale folder must be followed by `data` to count.
            let is_locale_folder = !segments[i].is_empty()
                && segments[i].chars().all(|c| c.is_ascii_digit())
                && segments
                    .get(i + 1)
                    .is_some_and(|s| s.eq_ignore_ascii_case("data"));
            let is_data_segment = segments[i].eq_ignore_ascii_case("data");
            if !(is_locale_folder || is_data_segment) {
                continue;
            }
            let strip: String = if i == 0 {
                String::new()
            } else {
                let mut s = segments[..i].join("/");
                s.push('/');
                s
            };
            return (strip, String::new());
        }
    }

    // No datadir anchor found. Heuristics for special cases.

    // Bare `*.rhm` at the root: a vanilla mission drop.
    if entries
        .iter()
        .any(|e| !e.is_empty() && !e.contains('/') && e.to_ascii_lowercase().ends_with(".rhm"))
    {
        return (String::new(), "data/levels/".to_string());
    }

    // `lib/` at root: the Spellforge lib folder, lands at Data/Levels/lib.
    if entries
        .iter()
        .any(|e| e.to_ascii_lowercase().starts_with("lib/"))
    {
        return (String::new(), "data/levels/".to_string());
    }

    // Last-resort fallback: treat the zip as a datadir root.  Anything
    // not matching standard paths will simply not be visible.
    (String::new(), String::new())
}

/// Resolve the overlay root for one exact selected mission entry.
///
/// Unlike [`detect_zip_layout`], this never lets archive ordering select a
/// language. `English/DATA/Levels/foo.rhm` selects the complete `English/`
/// datadir, while `foo/foo.rhm` treats the wrapping `foo/` directory as the
/// contents of `Data/Levels/`. The selected entry must be an exact safe path
/// advertised by the archive.
pub fn detect_zip_layout_for_mission(
    entries: impl IntoIterator<Item = impl AsRef<str>>,
    selected_rhm_entry: &str,
) -> Result<(String, String), String> {
    if selected_rhm_entry.is_empty()
        || selected_rhm_entry.starts_with('/')
        || selected_rhm_entry.contains('\\')
        || selected_rhm_entry
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        || !selected_rhm_entry.to_ascii_lowercase().ends_with(".rhm")
    {
        return Err(format!(
            "selected mission entry `{selected_rhm_entry}` is not a safe .rhm path"
        ));
    }

    let selected = selected_rhm_entry.to_ascii_lowercase();
    if !entries
        .into_iter()
        .any(|entry| entry.as_ref().replace('\\', "/").to_ascii_lowercase() == selected)
    {
        return Err(format!(
            "selected mission entry `{selected_rhm_entry}` is absent from the archive"
        ));
    }

    let segments = selected.split('/').collect::<Vec<_>>();
    if let Some(data_index) = segments
        .windows(2)
        .position(|pair| pair[0] == "data" && pair[1] == "levels")
    {
        let strip = if data_index == 0 {
            String::new()
        } else {
            format!("{}/", segments[..data_index].join("/"))
        };
        return Ok((strip, String::new()));
    }

    let strip = if segments.len() == 1 {
        String::new()
    } else {
        format!("{}/", segments[..segments.len() - 1].join("/"))
    };
    Ok((strip, "data/levels/".to_owned()))
}

pub struct SbFile {
    /// Game data is short and read sequentially / seekably, so we always
    /// slurp the whole file into memory and drive it with a `Cursor`.
    /// That lets `SbFile::open` uniformly consume bytes from the native
    /// filesystem *or* the shipping-datadir byte store hosted in
    /// `robin_util::asset_fs` without a type split.
    file: Cursor<AssetBytes>,
    size: u64,
    position: u64,
    last_error: i32,
    version: u32,
    /// Logical path requested by the caller. Typed legacy readers surface it
    /// in field-level parse errors even when bytes came from an overlay.
    path: String,
}

#[cfg(target_arch = "wasm32")]
pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_str()?;
    let normalised = path_str.replace('\\', "/");
    let path = Path::new(&normalised);
    // No `read_dir` on wasm, so we can't walk for case variants.
    // Shipping datadirs authored for wasm use exact-cased paths; a
    // single `asset_fs::exists` probe is enough.
    if robin_util::asset_fs::exists(path) {
        Some(path.to_path_buf())
    } else {
        None
    }
}

// Walks every component case-insensitively. Shipping datadirs use mixed
// casing across components (`DATA/` uppercase, `data/` lowercase), so
// case-folding has to apply to every component, not just the leaf.
// Dotfile entries (names starting with `.`) are skipped during the
// case-fold scan.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_str()?;
    if cfg!(windows) {
        // The case-fold walk below cannot rebuild drive/verbatim prefixes
        // (`C:\`, canonicalize's `\\?\C:\`), and Windows filesystems are
        // case-insensitive already, so a direct probe is both sufficient
        // and the only thing that works. Verbatim paths forbid forward
        // slashes, so fold separators to backslashes first.
        let backslashed = PathBuf::from(path_str.replace('/', "\\"));
        return backslashed.exists().then_some(backslashed);
    }
    let normalised = path_str.replace('\\', "/");
    let path = Path::new(&normalised);
    let mut components = path.components().peekable();
    let mut resolved = match components.peek() {
        Some(std::path::Component::RootDir) => {
            components.next();
            PathBuf::from("/")
        }
        _ => PathBuf::from("."),
    };
    for component in components {
        let target = component.as_os_str().to_str()?;
        let candidate = resolved.join(target);
        if candidate.exists() {
            resolved = candidate;
            continue;
        }
        let target_lower = target.to_ascii_lowercase();
        let mut found = false;
        if let Ok(entries) = fs::read_dir(&resolved) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && !name.starts_with('.')
                    && name.to_ascii_lowercase() == target_lower
                {
                    resolved = entry.path();
                    found = true;
                    break;
                }
            }
        }
        if !found {
            return None;
        }
    }
    Some(resolved)
}

/// Resolve a game-data path to an actual filesystem path.
///
/// Searches directory overlays, the selected and fallback locale roots, the
/// primary datadir, the direct path, and finally ordinary alternate paths.
/// Every native-filesystem probe is case-insensitive. Returns `None` if the
/// file cannot be found anywhere. Used by the video player to obtain a real
/// path for ffmpeg to open.
///
/// Zip overlays are *skipped* — they back the byte-buffer API only.
/// Callers that need a real filesystem path (the video player) won't
/// find zip-backed assets, which is correct: custom-mission mod data
/// never includes ffmpeg inputs.
pub fn resolve_data_path(path: &str) -> Option<PathBuf> {
    global_file_system().resolve_data_path(path)
}

impl SbFileSystem {
    pub fn resolve_data_dir_layers(&self, rel_dir: &str) -> Vec<PathBuf> {
        let normalised = rel_dir.replace('\\', "/");
        if let Some(root) = self.ranked_verifier_primary_path.lock().unwrap().clone() {
            let requested = Path::new(&normalised);
            if requested.is_absolute()
                || requested
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Vec::new();
            }
            return self
                .ranked_confined_candidates(&root, &normalised)
                .into_iter()
                .filter_map(|candidate| resolve_contained_directory(&root, &candidate))
                .collect();
        }
        let official_strict = self.official_projection_strict.load(Ordering::Acquire);
        let requested = Path::new(&normalised);
        if official_strict
            && (requested.is_absolute()
                || requested
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_))))
        {
            tracing::warn!("official projection rejected data-directory path {normalised:?}");
            return Vec::new();
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        {
            let overlays = self.overlay_paths.lock().unwrap();
            for overlay in overlays.iter().rev() {
                #[allow(irrefutable_let_patterns)] // wasm has no Zip variant
                if let OverlayRoot::Directory(dir) = overlay {
                    candidates.push(dir.join(&normalised));
                }
            }
        }
        let primary = self.primary_path.lock().unwrap().clone();
        for locale_root in self.locale_path_snapshot_for(&normalised) {
            if !Path::new(&locale_root).is_absolute()
                && let Some(primary) = &primary
            {
                candidates.push(primary.join(&locale_root).join(&normalised));
            }
            if !official_strict {
                candidates.push(Path::new(&locale_root).join(&normalised));
            }
        }
        let strict_locale = self.locale_paths().0.is_some() && is_required_locale_path(&normalised);
        if !strict_locale {
            if let Some(primary) = &primary {
                candidates.push(primary.join(&normalised));
            }
            if !official_strict {
                candidates.push(PathBuf::from(&normalised));
            }
            for alt in self.alternate_paths.lock().unwrap().iter() {
                if let Some(primary) = &primary {
                    candidates.push(primary.join(alt).join(&normalised));
                }
                if !official_strict {
                    candidates.push(Path::new(alt).join(&normalised));
                }
            }
        }
        candidates
            .into_iter()
            .map(|dir| self.physical_path(&dir))
            .filter_map(|dir| {
                if dir.is_dir() {
                    return Some(dir);
                }
                resolve_case_insensitive(&dir).filter(|p| p.is_dir())
            })
            .collect()
    }

    pub fn resolve_data_path(&self, path: &str) -> Option<PathBuf> {
        let normalised = path.replace('\\', "/");
        let p = Path::new(&normalised);
        if let Some(root) = self.ranked_verifier_primary_path.lock().unwrap().clone() {
            if p.is_absolute()
                || p.components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return None;
            }
            return self
                .ranked_confined_candidates(&root, &normalised)
                .into_iter()
                .find_map(|candidate| resolve_contained_file(&root, &candidate));
        }
        let official_strict = self.official_projection_strict.load(Ordering::Acquire);
        if official_strict && p.is_absolute() {
            tracing::warn!("official projection rejected absolute data path {normalised:?}");
            return None;
        }
        if !p.is_absolute()
            && p.components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            tracing::warn!("resolve_data_path: rejected escaping path {normalised}");
            return None;
        }

        // Overlay paths intentionally take precedence over the primary datadir.
        let overlay_paths = self.overlay_paths.lock().unwrap();
        // Overlays form a stack: the most recently mounted mission/package
        // must win over the core overlay and over any shared library mounted
        // underneath it.  Walking in insertion order made a local/core file
        // with the same virtual path silently replace authenticated
        // distributed-mod bytes.
        for overlay in overlay_paths.iter().rev() {
            let OverlayRoot::Directory(dir) = overlay else {
                continue;
            };
            let full = dir.join(&normalised);
            if let Some(resolved) = resolve_contained_file(dir, &full) {
                return Some(resolved);
            }
        }
        drop(overlay_paths);

        let primary = self.primary_path.lock().unwrap().clone();
        for locale_root in self.locale_path_snapshot_for(&normalised) {
            if !Path::new(&locale_root).is_absolute()
                && let Some(primary) = &primary
            {
                let full = primary.join(&locale_root).join(&normalised);
                if let Some(resolved) = resolve_contained_file(primary, &full) {
                    return Some(resolved);
                }
            }
            if !official_strict {
                let full = Path::new(&locale_root).join(&normalised);
                if let Some(resolved) = resolve_case_insensitive(&self.physical_path(&full))
                    && resolved.is_file()
                {
                    return Some(resolved);
                }
            }
        }

        if self.locale_paths().0.is_some() && is_required_locale_path(&normalised) {
            return None;
        }

        if let Some(primary) = &primary {
            let full = primary.join(&normalised);
            if let Some(resolved) = resolve_contained_file(primary, &full) {
                return Some(resolved);
            }
        }

        // Direct path
        if !official_strict
            && let Some(resolved) = resolve_case_insensitive(&self.physical_path(p))
            && resolved.is_file()
        {
            return Some(resolved);
        }

        // Alternate paths
        let alt_paths = self.alternate_paths.lock().unwrap();
        for alt in alt_paths.iter() {
            if let Some(primary) = &primary {
                let full = primary.join(alt).join(&normalised);
                if let Some(resolved) = resolve_contained_file(primary, &full) {
                    return Some(resolved);
                }
            }
            if !official_strict {
                let full = format!("{}/{}", alt, normalised);
                if let Some(resolved) =
                    resolve_case_insensitive(&self.physical_path(Path::new(&full)))
                    && resolved.is_file()
                {
                    return Some(resolved);
                }
            }
        }

        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_contained_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    let resolved = fs::canonicalize(resolved).ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_contained_directory(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    let resolved = fs::canonicalize(resolved).ok()?;
    (resolved.starts_with(root) && resolved.is_dir()).then_some(resolved)
}

#[cfg(not(target_arch = "wasm32"))]
fn path_exists_contained(root: &Path, candidate: &Path) -> Result<bool, i32> {
    let Some(resolved) = resolve_case_insensitive(candidate) else {
        return Ok(false);
    };
    let resolved = fs::canonicalize(&resolved).map_err(|error| {
        tracing::warn!("asset {} cannot be resolved: {error}", resolved.display());
        SBFILE_ERROR_READ
    })?;
    if !resolved.starts_with(root) {
        tracing::warn!(
            "asset {} escapes mount {}",
            resolved.display(),
            root.display()
        );
        return Err(SBFILE_ERROR_READ);
    }
    Ok(true)
}

#[cfg(target_arch = "wasm32")]
fn resolve_contained_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    (resolved.starts_with(root)).then_some(resolved)
}

#[cfg(target_arch = "wasm32")]
fn resolve_contained_directory(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    (resolved.starts_with(root) && resolved.is_dir()).then_some(resolved)
}

#[cfg(target_arch = "wasm32")]
fn path_exists_contained(root: &Path, candidate: &Path) -> Result<bool, i32> {
    Ok(resolve_case_insensitive(candidate).is_some_and(|resolved| resolved.starts_with(root)))
}

/// Read `path` as bytes, honouring case-insensitive resolution on native
/// for datadirs that use mixed case on the wire (e.g. demo installers
/// ship `DATA/` uppercase).
///
/// Per-path NotFound logs at `trace` (expected fallthrough during
/// alternate-path search); any *other* error (network failure, HTTP
/// 5xx, permission denied) is a real problem and logs at `warn` —
/// silently swallowing those turned a wasm network blip into "file
/// missing" and cost us an afternoon of debugging.
///
/// Note: the original release also fired a file-not-found callback on
/// miss to drive an "insert CD" disc-swap prompt. The Rust port ships
/// from a flat datadir, has no CD-media support, and therefore has no
/// equivalent — intentionally dropped.
fn try_read(file_system: &SbFileSystem, path: &str) -> Result<Option<AssetBytes>, i32> {
    match file_system.assets.read_shared(path) {
        Ok(bytes) => return Ok(Some(bytes)),
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => {
            tracing::trace!("asset {path}: not found");
        }
        // Absolute host paths are intentionally handled by the compatibility
        // fallback below; virtual paths must remain mount-contained.
        Err(robin_util::asset_fs::AssetError::InvalidPath(_)) if Path::new(path).is_absolute() => {}
        Err(e) => {
            tracing::warn!("asset read failed for {path}: {e}");
            return Err(SBFILE_ERROR_READ);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(resolved) = file_system.resolve_instance_path(Path::new(path))? {
        // Physical fallback must not re-enter the process-global VFS.
        match fs::read(&resolved) {
            Ok(bytes) => return Ok(Some(AssetBytes::from(bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::trace!(
                    "asset {} (case-resolved from {path}): not found",
                    resolved.display()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "asset read failed for {} (case-resolved from {path}): {e}",
                    resolved.display()
                );
                return Err(SBFILE_ERROR_READ);
            }
        }
    }
    Ok(None)
}

impl SbFile {
    pub fn open(path: &str, _flags: i32) -> Result<Self, i32> {
        global_file_system().open(path, _flags)
    }
}

impl SbFileSystem {
    pub fn open(&self, path: &str, _flags: i32) -> Result<SbFile, i32> {
        let normalised = path.replace('\\', "/");
        let requested = Path::new(&normalised);
        if let Some(root) = self.ranked_verifier_primary_path.lock().unwrap().clone() {
            if requested.is_absolute()
                || requested
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(SBFILE_ERROR_READ);
            }
            let resolved = self
                .ranked_confined_candidates(&root, &normalised)
                .into_iter()
                .find_map(|candidate| resolve_contained_file(&root, &candidate))
                .ok_or(SBFILE_ERROR_FILE_NOT_FOUND)?;
            let bytes = fs::read(&resolved).map_err(|error| {
                tracing::warn!(
                    "ranked verifier asset {} cannot be read: {error}",
                    resolved.display()
                );
                SBFILE_ERROR_READ
            })?;
            return Ok(SbFile::from_bytes(bytes, normalised));
        }
        let official_strict = self.official_projection_strict.load(Ordering::Acquire);
        if requested.is_absolute() {
            if official_strict {
                tracing::warn!("official projection rejected absolute open path {normalised:?}");
                return Err(SBFILE_ERROR_READ);
            }
            return try_read(self, &normalised)?
                .map(|bytes| SbFile::from_bytes(bytes, normalised.clone()))
                .ok_or(SBFILE_ERROR_FILE_NOT_FOUND);
        }
        if requested
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            tracing::warn!("SbFile::open: rejected escaping path {normalised}");
            return Err(SBFILE_ERROR_READ);
        }
        let overlay_paths = self.overlay_paths.lock().unwrap();
        for overlay in overlay_paths.iter().rev() {
            if let Some(bytes) = read_from_overlay(self, overlay, &normalised)? {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
        }
        drop(overlay_paths);

        let primary = self.primary_path.lock().unwrap().clone();
        for locale_root in self.locale_path_snapshot_for(&normalised) {
            if !Path::new(&locale_root).is_absolute()
                && let Some(primary) = &primary
                && let Some(bytes) = try_read(
                    self,
                    &primary
                        .join(&locale_root)
                        .join(&normalised)
                        .to_string_lossy(),
                )?
            {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
            if !official_strict
                && let Some(bytes) = try_read(
                    self,
                    &Path::new(&locale_root).join(&normalised).to_string_lossy(),
                )?
            {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
        }

        if self.locale_paths().0.is_some() && is_required_locale_path(&normalised) {
            tracing::warn!(
                "SbFile::open: required localized asset {normalised} is absent from the selected pack"
            );
            return Err(SBFILE_ERROR_FILE_NOT_FOUND);
        }

        if let Some(primary) = &primary
            && let Some(bytes) = try_read(self, &primary.join(&normalised).to_string_lossy())?
        {
            return Ok(SbFile::from_bytes(bytes, normalised.clone()));
        }
        if official_strict {
            // Shipping archives install authenticated decoded bundles into
            // the VFS after the initially empty authority snapshot.
            match self.assets.read_shared(&normalised) {
                Ok(bytes) => return Ok(SbFile::from_bytes(bytes, normalised.clone())),
                Err(robin_util::asset_fs::AssetError::NotFound(_)) => {}
                Err(error) => {
                    tracing::warn!("official asset read failed for {normalised}: {error}");
                    return Err(SBFILE_ERROR_READ);
                }
            }
            tracing::warn!(
                "SbFile::open: {normalised} not found inside the sealed official mounts"
            );
            return Err(SBFILE_ERROR_FILE_NOT_FOUND);
        }
        if let Some(bytes) = try_read(self, &normalised)? {
            return Ok(SbFile::from_bytes(bytes, normalised.clone()));
        }
        let alt_paths = self.alternate_paths.lock().unwrap();
        for alt in alt_paths.iter() {
            if let Some(primary) = &primary
                && let Some(bytes) =
                    try_read(self, &primary.join(alt).join(&normalised).to_string_lossy())?
            {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
            if let Some(bytes) = try_read(self, &format!("{alt}/{normalised}"))? {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
        }
        tracing::warn!(
            "SbFile::open: {normalised} not found (tried {} locale + direct + {} alternate paths)",
            self.locale_path_snapshot_for(&normalised).len(),
            alt_paths.len(),
        );
        Err(SBFILE_ERROR_FILE_NOT_FOUND)
    }

    /// Read through the same overlay, locale, and confinement rules as `open`,
    /// retaining shared backing storage for memory-mounted files.
    pub fn read_shared(&self, path: &str) -> Result<AssetBytes, i32> {
        Ok(self.open(path, SB_FILE_READ)?.into_shared_bytes())
    }

    pub fn read_all(&self, path: &str) -> Result<Vec<u8>, i32> {
        self.read_shared(path).map(AssetBytes::into_vec)
    }
}

impl SbFile {
    /// Construct a read-only legacy stream from already owned bytes.
    ///
    /// Embedded Original checkpoints (for example schema-11 parity traces)
    /// should not need a temporary filesystem round trip merely to use the
    /// same positional/versioned reader as on-disk saves. `display_path` is
    /// retained only for structured diagnostics.
    pub fn from_owned_bytes(bytes: Vec<u8>, display_path: impl Into<String>) -> Self {
        Self::from_bytes(bytes, display_path.into())
    }

    fn from_bytes(bytes: impl Into<AssetBytes>, path: String) -> Self {
        let bytes = bytes.into();
        let size = bytes.len() as u64;
        SbFile {
            file: Cursor::new(bytes),
            size,
            position: 0,
            last_error: SBFILE_NO_ERROR,
            version: 0,
            path,
        }
    }

    pub fn read_all(path: &str) -> Result<Vec<u8>, i32> {
        global_file_system().read_all(path)
    }

    /// Consume the stream and retain its full backing buffer, irrespective of
    /// the current cursor position. Shared memory-mounted bytes are not copied.
    pub fn into_shared_bytes(self) -> AssetBytes {
        self.file.into_inner()
    }

    /// Consume the stream as an owned buffer. Unique storage is transferred;
    /// memory-mounted storage is copied if other readers still share it.
    /// Read-only consumers should prefer [`Self::into_shared_bytes`].
    pub fn into_bytes(self) -> Vec<u8> {
        self.into_shared_bytes().into_vec()
    }

    pub fn read(&mut self, buf: &mut [u8]) -> i32 {
        match self.file.read_exact(buf) {
            Ok(()) => {
                self.position += buf.len() as u64;
                self.last_error = SBFILE_NO_ERROR;
                SBFILE_NO_ERROR
            }
            Err(_) => {
                self.last_error = SBFILE_ERROR_READ;
                SBFILE_ERROR_READ
            }
        }
    }

    pub fn skip(&mut self, distance: i64, mode: u32) -> i32 {
        let seek_from = match mode {
            0 => SeekFrom::Start(distance as u64),
            1 => SeekFrom::Current(distance),
            2 => SeekFrom::End(distance),
            other => {
                tracing::warn!("SbFile::skip: unknown mode {other}, falling back to SEEK_CUR");
                SeekFrom::Current(distance)
            }
        };
        match self.file.seek(seek_from) {
            Ok(pos) => {
                self.position = pos;
                self.last_error = SBFILE_NO_ERROR;
                SBFILE_NO_ERROR
            }
            Err(_) => {
                self.last_error = SBFILE_ERROR_SEEK;
                SBFILE_ERROR_SEEK
            }
        }
    }

    pub fn tell(&mut self) -> u64 {
        self.file.stream_position().unwrap_or(self.position)
    }
    pub fn get_size(&self) -> u64 {
        self.size
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    /// True once the cursor has reached the end of the in-memory buffer.
    pub fn is_eof(&self) -> bool {
        self.position >= self.size
    }
    pub fn is_read_mode(&self) -> bool {
        true
    }
    pub fn is_write_mode(&self) -> bool {
        false
    }
    pub fn get_version(&self) -> u32 {
        self.version
    }

    // ── Binary readers ───────────────────────────────────────────

    // TODO(legacy-io): once level and sprite authored-data consumers use
    // LegacyReader, make these mutate-in-place/status-code methods private.

    pub fn serialize_bytes(&mut self, buf: &mut [u8]) -> Result<(), i32> {
        if self.read(buf) < 0 {
            Err(self.last_error)
        } else {
            Ok(())
        }
    }
    pub fn serialize_u8(&mut self, val: &mut u8) -> Result<(), i32> {
        let mut b = [0u8; 1];
        self.serialize_bytes(&mut b)?;
        *val = b[0];
        Ok(())
    }
    pub fn serialize_i8(&mut self, val: &mut i8) -> Result<(), i32> {
        let mut b = 0u8;
        self.serialize_u8(&mut b)?;
        *val = b as i8;
        Ok(())
    }
    pub fn serialize_u16(&mut self, val: &mut u16) -> Result<(), i32> {
        let mut b = [0u8; 2];
        self.serialize_bytes(&mut b)?;
        *val = u16::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_i16(&mut self, val: &mut i16) -> Result<(), i32> {
        let mut b = [0u8; 2];
        self.serialize_bytes(&mut b)?;
        *val = i16::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_u32(&mut self, val: &mut u32) -> Result<(), i32> {
        let mut b = [0u8; 4];
        self.serialize_bytes(&mut b)?;
        *val = u32::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_i32(&mut self, val: &mut i32) -> Result<(), i32> {
        let mut b = [0u8; 4];
        self.serialize_bytes(&mut b)?;
        *val = i32::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_u64(&mut self, val: &mut u64) -> Result<(), i32> {
        let mut b = [0u8; 8];
        self.serialize_bytes(&mut b)?;
        *val = u64::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_i64(&mut self, val: &mut i64) -> Result<(), i32> {
        let mut b = [0u8; 8];
        self.serialize_bytes(&mut b)?;
        *val = i64::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_f32(&mut self, val: &mut f32) -> Result<(), i32> {
        let mut b = [0u8; 4];
        self.serialize_bytes(&mut b)?;
        *val = f32::from_le_bytes(b);
        Ok(())
    }
    pub fn serialize_bool(&mut self, val: &mut bool) -> Result<(), i32> {
        let mut b = 0u8;
        self.serialize_u8(&mut b)?;
        *val = b != 0;
        Ok(())
    }
    pub fn serialize_version(&mut self) -> Result<(), i32> {
        let mut v = 0u32;
        self.serialize_u32(&mut v)?;
        self.version = v;
        Ok(())
    }
    pub fn serialize_string(&mut self, s: &mut String) -> Result<(), i32> {
        let mut len = 0u16;
        self.serialize_u16(&mut len)?;
        let mut bytes = vec![0u8; len as usize];
        self.serialize_bytes(&mut bytes)?;
        *s = String::from_utf8_lossy(&bytes).into_owned();
        Ok(())
    }

    pub fn checkpoint(&mut self) -> Result<(), i32> {
        let mut m = 0u16;
        self.serialize_u16(&mut m)?;
        if m != 0x7777 {
            tracing::warn!("CHECKPOINT: shifted (0x{:04x})", m);
            return Err(SBFILE_ERROR_READ);
        }
        Ok(())
    }

    pub fn exists(path: &str) -> bool {
        match global_file_system().try_exists(path) {
            Ok(exists) => exists,
            Err(error) => {
                tracing::warn!("SbFile::exists({path}): lookup failed with error {error}");
                false
            }
        }
    }

    pub fn add_alternate_path(path: &str) -> i32 {
        global_file_system().add_alternate_path(path)
    }

    /// Atomically replace the selected locale root and fallback locale root.
    ///
    /// For presentation asset families, locale roots are searched after
    /// overlays but before the primary/direct datadir and ordinary alternate
    /// paths. Simulation inputs never consult this layer. A relative root is
    /// resolved both beneath the primary datadir and relative to the process
    /// working directory, matching the established alternate-path
    /// conventions. Passing `None` disables that locale layer.
    pub fn set_locale_paths(selected: Option<&str>, fallback: Option<&str>) -> i32 {
        global_file_system().set_locale_paths(selected, fallback)
    }

    /// Return one coherent snapshot of the selected and fallback locale roots.
    pub fn locale_paths() -> (Option<String>, Option<String>) {
        global_file_system().locale_paths()
    }

    pub fn add_overlay_path(path: &str) -> i32 {
        global_file_system().add_overlay_path(path)
    }

    /// Mount a zip archive as an overlay root, with no on-disk extraction.
    ///
    /// The archive is held open for the lifetime of the overlay; its
    /// internal layout is auto-detected (see `detect_zip_layout`) so the
    /// engine can look up `Data/Levels/foo.rhm` regardless of whether the
    /// zip wraps that path inside `English/` or stores `foo.rhm` bare at
    /// the root.
    ///
    /// `remove_overlay(zip_path)` undoes this.
    pub fn add_overlay_zip(zip_path: &str) -> i32 {
        global_file_system().add_overlay_zip(zip_path)
    }

    /// Mount a zip using one exact `.rhm` entry to select its language or
    /// folder-wrapped datadir root.
    pub fn add_overlay_zip_for_mission(zip_path: &str, rhm_entry: &str) -> i32 {
        global_file_system().add_overlay_zip_for_mission(zip_path, rhm_entry)
    }

    /// Mount exact already-validated ZIP bytes without consulting a host
    /// filesystem. Browser multiplayer uses this for the complete mission and
    /// shared Spellforge library distributed by the authenticated host.
    pub fn add_overlay_zip_bytes_for_mission(
        mount_id: &str,
        bytes: Arc<[u8]>,
        rhm_entry: Option<&str>,
    ) -> i32 {
        global_file_system().add_overlay_zip_bytes_for_mission(mount_id, bytes, rhm_entry)
    }

    /// Remove an overlay by its registered path (works for both directory
    /// and zip overlays).  Returns `SBFILE_ERROR_PATH_NOT_IN_SET` if not
    /// found.
    pub fn remove_overlay(path: &str) -> i32 {
        global_file_system().remove_overlay(path)
    }

    /// Returns all directory-overlay paths (in priority order).  Zip
    /// overlays are intentionally excluded: this API exists for callers
    /// that want to walk the directory tree (e.g. enumerate
    /// `Data/Characters/*.rhs.d/`), which doesn't apply to in-memory zip
    /// roots.
    pub fn overlay_paths() -> Vec<String> {
        global_file_system().overlay_paths()
    }

    /// Whether an in-memory ZIP overlay is active. Callers that require a
    /// filesystem-verifiable content closure must fail instead of silently
    /// omitting these roots from [`Self::overlay_paths`].
    pub fn has_zip_overlays() -> bool {
        global_file_system().has_zip_overlays()
    }

    pub fn set_primary_path(path: &str) -> i32 {
        global_file_system().set_primary_path(path)
    }

    /// Permanently confine this process's legacy asset resolver to one
    /// canonical datadir. This is intentionally one-way: the isolated ranked
    /// verifier child handles exactly one job and must never regain ambient
    /// overlay, locale, VFS, current-directory, or alternate-path lookup.
    pub fn lock_ranked_verifier_primary_path(path: &Path) -> i32 {
        global_file_system().lock_ranked_verifier_primary_path(path)
    }

    /// Permanently confine the ranked verifier to one canonical datadir and
    /// one exact locale directory beneath it. No fallback locale or ambient
    /// lookup path is retained.
    pub fn lock_ranked_verifier_primary_path_with_locale(
        path: &Path,
        resource_locale_root: &str,
    ) -> i32 {
        global_file_system()
            .lock_ranked_verifier_primary_path_with_locale(path, resource_locale_root)
    }

    pub fn mount_snapshot() -> SbFileMountSnapshot {
        global_file_system().mount_snapshot()
    }

    /// Explicit legacy host-preparation bridge. Resource execution should
    /// retain the returned instance instead of consulting global mounts.
    pub fn snapshot_legacy_file_system() -> SbFileSystem {
        global_file_system().snapshot()
    }

    pub fn official_projection_lookup_is_strict() -> bool {
        global_file_system()
            .official_projection_strict
            .load(Ordering::Acquire)
    }

    /// Atomically install the only source, locale, and built-in overlay roots
    /// visible to an official projection process.
    pub fn configure_official_projection_mounts(
        source_root: &Path,
        resource_locale_root: &str,
        core_overlay_root: &Path,
    ) -> Result<(), String> {
        global_file_system().configure_official_projection_mounts(
            source_root,
            resource_locale_root,
            core_overlay_root,
        )
    }

    /// Resolve a relative datadir *directory* to every existing native
    /// directory across the search order (overlays, selected locale, fallback
    /// locale, primary, direct, then ordinary alternates), case-insensitively.
    ///
    /// Callers that stat many files under one datadir directory resolve
    /// the layering once with this instead of paying the full per-file
    /// search for each name. Zip overlays have no native directories and
    /// are skipped, matching [`SbFile::overlay_paths`].
    pub fn resolve_data_dir_layers(rel_dir: &str) -> Vec<PathBuf> {
        global_file_system().resolve_data_dir_layers(rel_dir)
    }

    pub fn remove_alternate_path(path: &str) -> i32 {
        global_file_system().remove_alternate_path(path)
    }
}

impl SbFileSystem {
    /// Freeze lookup configuration for a prepared resource environment.
    /// Call at the host's mount-change boundary, not concurrently with mount
    /// publication. Concurrent configuration mutation during capture is unsupported:
    /// the individual configuration locks do not form a multi-field transaction.
    /// After capture, mutable mount/locale/VFS selections are not shared.
    /// This pins configuration and immutable in-memory archive bytes, not
    /// every native disk byte; native content may still require digest checks.
    /// Native relative lookup pins the current working directory as well.
    /// Panics if the host cannot capture that directory (no ambient fallback).
    pub fn snapshot(&self) -> Self {
        Self {
            origin_identity: self.origin_identity,
            #[cfg(not(target_arch = "wasm32"))]
            working_directory: Some(self.working_directory.clone().unwrap_or_else(|| {
                std::env::current_dir().expect("cannot capture prepared resource working directory")
            })),
            assets: Arc::new(self.assets.snapshot()),
            alternate_paths: Mutex::new(self.alternate_paths.lock().unwrap().clone()),
            locale_paths: Mutex::new(self.locale_paths.lock().unwrap().clone()),
            overlay_paths: Mutex::new(self.overlay_paths.lock().unwrap().clone()),
            primary_path: Mutex::new(self.primary_path.lock().unwrap().clone()),
            ranked_verifier_primary_path: Mutex::new(
                self.ranked_verifier_primary_path.lock().unwrap().clone(),
            ),
            official_projection_strict: AtomicBool::new(
                self.official_projection_strict.load(Ordering::Acquire),
            ),
        }
    }

    pub fn mount_snapshot(&self) -> SbFileMountSnapshot {
        let (selected_locale, fallback_locale) = self.locale_paths();
        SbFileMountSnapshot {
            #[cfg(not(target_arch = "wasm32"))]
            working_directory: self.working_directory.clone(),
            #[cfg(target_arch = "wasm32")]
            working_directory: None,
            alternate_paths: self.alternate_paths.lock().unwrap().clone(),
            selected_locale,
            fallback_locale,
            overlay_paths: self
                .overlay_paths
                .lock()
                .unwrap()
                .iter()
                .map(|overlay| overlay.display_path().into_owned())
                .collect(),
            primary_path: self.primary_path.lock().unwrap().clone(),
            ranked_verifier_primary_path: self.ranked_verifier_primary_path.lock().unwrap().clone(),
            asset_vfs: self.assets.authority_snapshot(),
            official_projection_strict: self.official_projection_strict.load(Ordering::Acquire),
        }
    }

    pub fn configure_official_projection_mounts(
        &self,
        source_root: &Path,
        resource_locale_root: &str,
        core_overlay_root: &Path,
    ) -> Result<(), String> {
        fn exact_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
            if !path.is_absolute() {
                return Err(format!("{label} must be absolute: {}", path.display()));
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "{label} must be a non-symlink directory: {}",
                    path.display()
                ));
            }
            let canonical = fs::canonicalize(path).map_err(|error| {
                format!("cannot canonicalize {label} {}: {error}", path.display())
            })?;
            if canonical != path {
                return Err(format!(
                    "{label} must be normalized (expected {}): {}",
                    canonical.display(),
                    path.display()
                ));
            }
            Ok(canonical)
        }

        if resource_locale_root.is_empty()
            || !resource_locale_root
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            return Err("official projection LCID must be one numeric component".to_owned());
        }
        let source_root = exact_directory(source_root, "official source root")?;
        let core_overlay_root = exact_directory(core_overlay_root, "official core overlay root")?;
        let snapshot = self.mount_snapshot();
        if !snapshot.alternate_paths.is_empty()
            || snapshot.selected_locale.is_some()
            || snapshot.fallback_locale.is_some()
            || self.presentation_locale().is_some()
            || !snapshot.overlay_paths.is_empty()
            || snapshot.primary_path.is_some()
            || !snapshot.asset_vfs.is_empty()
            || snapshot.official_projection_strict
        {
            return Err(format!(
                "filesystem/VFS authorities were installed before official projection: {snapshot:?}"
            ));
        }
        self.official_projection_strict
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "official projection lookup mode was already configured".to_owned())?;
        *self.primary_path.lock().unwrap() = Some(source_root);
        *self.locale_paths.lock().unwrap() = LocaleLookup {
            selected: Some(resource_locale_root.to_owned()),
            ..LocaleLookup::default()
        };
        self.overlay_paths
            .lock()
            .unwrap()
            .push(OverlayRoot::Directory(core_overlay_root));
        Ok(())
    }

    fn physical_path(&self, path: &Path) -> PathBuf {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(root) = &self.working_directory {
            return root.join(path);
        }
        path.to_path_buf()
    }

    fn resolve_instance_path(&self, path: &Path) -> Result<Option<PathBuf>, i32> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(resolve_case_insensitive(&self.physical_path(path)))
        }
        #[cfg(target_arch = "wasm32")]
        {
            match self.assets.try_exists(path) {
                Ok(exists) => Ok(exists.then(|| path.to_path_buf())),
                Err(robin_util::asset_fs::AssetError::InvalidPath(_)) => Ok(None),
                Err(error) => {
                    tracing::warn!(
                        "instance asset existence failed for {}: {error}",
                        path.display()
                    );
                    Err(SBFILE_ERROR_READ)
                }
            }
        }
    }

    pub fn try_exists(&self, path: &str) -> Result<bool, i32> {
        let normalised = path.replace('\\', "/");
        let requested = Path::new(&normalised);
        if let Some(root) = self.ranked_verifier_primary_path.lock().unwrap().clone() {
            if requested.is_absolute()
                || requested
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(SBFILE_ERROR_READ);
            }
            for candidate in self.ranked_confined_candidates(&root, &normalised) {
                if path_exists_contained(&root, &candidate)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let official_strict = self.official_projection_strict.load(Ordering::Acquire);
        if official_strict && requested.is_absolute() {
            tracing::warn!("official projection rejected absolute existence path {normalised:?}");
            return Err(SBFILE_ERROR_READ);
        }
        if !requested.is_absolute()
            && requested
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(SBFILE_ERROR_READ);
        }

        if !requested.is_absolute() {
            let overlays = self.overlay_paths.lock().unwrap();
            for overlay in overlays.iter().rev() {
                match overlay {
                    OverlayRoot::Directory(root) => {
                        if path_exists_contained(root, &root.join(&normalised))? {
                            return Ok(true);
                        }
                    }
                    OverlayRoot::Zip(zip) if zip.exists(&normalised) => return Ok(true),
                    OverlayRoot::Zip(_) => {}
                }
            }
        }

        let primary = self.primary_path.lock().unwrap().clone();
        if !requested.is_absolute() {
            for locale_root in self.locale_path_snapshot_for(&normalised) {
                if !Path::new(&locale_root).is_absolute()
                    && let Some(primary) = &primary
                    && path_exists_contained(
                        primary,
                        &primary.join(&locale_root).join(&normalised),
                    )?
                {
                    return Ok(true);
                }
                if !official_strict
                    && self
                        .resolve_instance_path(&Path::new(&locale_root).join(&normalised))?
                        .is_some()
                {
                    return Ok(true);
                }
            }

            if self.locale_paths().0.is_some() && is_required_locale_path(&normalised) {
                return Ok(false);
            }

            if let Some(primary) = &primary
                && path_exists_contained(primary, &primary.join(&normalised))?
            {
                return Ok(true);
            }
        }
        match self.assets.try_exists(requested) {
            Ok(true) => return Ok(true),
            Ok(false) | Err(robin_util::asset_fs::AssetError::InvalidPath(_)) => {}
            Err(error) => {
                tracing::warn!("SbFileSystem::try_exists({normalised}): {error}");
                return Err(SBFILE_ERROR_READ);
            }
        }
        if !official_strict && self.resolve_instance_path(requested)?.is_some() {
            return Ok(true);
        }

        if !requested.is_absolute() {
            let alternate_paths = self.alternate_paths.lock().unwrap();
            for alternate in alternate_paths.iter() {
                if let Some(primary) = &primary
                    && path_exists_contained(primary, &primary.join(alternate).join(&normalised))?
                {
                    return Ok(true);
                }
                if !official_strict
                    && self
                        .resolve_instance_path(&Path::new(alternate).join(&normalised))?
                        .is_some()
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn add_alternate_path(&self, path: &str) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected alternate path {path:?}");
            return SBFILE_ERROR_READ;
        }
        let mut paths = self.alternate_paths.lock().unwrap();
        if paths.iter().any(|candidate| candidate == path) {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        paths.push(path.to_string());
        self.assets.invalidate_content(false);
        SBFILE_NO_ERROR
    }

    /// Atomically replace both locale roots without disturbing generic
    /// alternate paths. Invalid roots reject the entire update, leaving the
    /// previous state intact. Raw root configuration clears any previous
    /// presentation language; applications use `set_presentation_locale`.
    pub fn set_locale_paths(&self, selected: Option<&str>, fallback: Option<&str>) -> i32 {
        self.set_presentation_locale(selected, fallback, None)
    }

    /// Install the application-selected BCP-47 language and its resource roots
    /// together. Language-only changes invalidate presentation caches, even
    /// when roots are unchanged (for example embedded shipping packs).
    /// Like other mount updates, call at the application's preparation boundary.
    pub fn set_presentation_locale(
        &self,
        selected: Option<&str>,
        fallback: Option<&str>,
        language: Option<&str>,
    ) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected locale-path mutation");
            return SBFILE_ERROR_READ;
        }
        let selected = match selected.map(normalise_locale_root).transpose() {
            Ok(path) => path,
            Err(error) => return error,
        };
        let fallback = match fallback.map(normalise_locale_root).transpose() {
            Ok(path) => path,
            Err(error) => return error,
        };
        let mut locale_paths = self.locale_paths.lock().unwrap();
        *locale_paths = LocaleLookup {
            selected,
            fallback,
            language: language.map(str::to_owned),
        };
        self.assets.invalidate_content(true);
        SBFILE_NO_ERROR
    }

    pub fn locale_paths(&self) -> (Option<String>, Option<String>) {
        let locale = self.locale_paths.lock().unwrap();
        (locale.selected.clone(), locale.fallback.clone())
    }

    /// Language of this reader's presentation resources, not process state.
    pub fn presentation_locale(&self) -> Option<String> {
        self.locale_paths.lock().unwrap().language.clone()
    }

    fn ranked_confined_candidates(&self, root: &Path, normalised: &str) -> Vec<PathBuf> {
        let mut candidates = Vec::with_capacity(2);
        if is_locale_overlay_path(normalised)
            && let Some(locale) = self.locale_paths().0
        {
            candidates.push(root.join(locale).join(normalised));
            if is_required_locale_path(normalised) {
                return candidates;
            }
        }
        candidates.push(root.join(normalised));
        candidates
    }

    /// Snapshot the roots in lookup order, suppressing a duplicate fallback.
    fn locale_path_snapshot_for(&self, path: &str) -> Vec<String> {
        // A language pack is presentation data. Never allow an installed
        // locale directory to replace levels, scripts, gameplay profiles, or
        // any other simulation input merely because it contains a matching
        // Data/ subtree.
        if !is_locale_overlay_path(path) {
            return Vec::new();
        }
        let (selected, fallback) = self.locale_paths();
        let mut paths = Vec::with_capacity(2);
        if let Some(selected) = selected {
            paths.push(selected);
        }
        if let Some(fallback) = fallback
            && is_optional_english_fallback_path(path)
            && !paths
                .iter()
                .any(|selected| selected.eq_ignore_ascii_case(&fallback))
        {
            paths.push(fallback);
        }
        paths
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_overlay_path(&self, path: &str) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected overlay path {path:?}");
            return SBFILE_ERROR_READ;
        }
        let canonical = match fs::canonicalize(path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                tracing::warn!("SbFileSystem::add_overlay_path: {path} is not a directory");
                return SBFILE_ERROR_NO_FILE;
            }
            Err(error) => {
                tracing::warn!("SbFileSystem::add_overlay_path: cannot open {path}: {error}");
                return SBFILE_ERROR_FILE_NOT_FOUND;
            }
        };
        let mut paths = self.overlay_paths.lock().unwrap();
        if paths
            .iter()
            .any(|candidate| candidate.display_path() == canonical.to_string_lossy())
        {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        paths.push(OverlayRoot::Directory(canonical));
        self.assets.invalidate_content(false);
        SBFILE_NO_ERROR
    }

    #[cfg(target_arch = "wasm32")]
    pub fn add_overlay_path(&self, _path: &str) -> i32 {
        // TODO(asset-vfs): add a browser-provided directory mount if wasm
        // gains a host filesystem abstraction. Returning an explicit error
        // avoids pretending the mount was installed.
        SBFILE_ERROR_NO_FILE
    }

    pub fn add_overlay_zip(&self, zip_path: &str) -> i32 {
        self.add_overlay_zip_inner(zip_path, None)
    }

    pub fn add_overlay_zip_for_mission(&self, zip_path: &str, rhm_entry: &str) -> i32 {
        self.add_overlay_zip_inner(zip_path, Some(rhm_entry))
    }

    fn add_overlay_zip_inner(&self, zip_path: &str, rhm_entry: Option<&str>) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            tracing::warn!("ranked verifier filesystem rejected overlay zip {zip_path:?}");
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected overlay zip {zip_path:?}");
            return SBFILE_ERROR_READ;
        }
        #[allow(unused_mut)]
        let mut paths = self.overlay_paths.lock().unwrap();
        if paths
            .iter()
            .any(|candidate| candidate.display_path() == zip_path)
        {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let overlay = match ZipOverlay::open_for_mission(Path::new(zip_path), rhm_entry) {
            Ok(overlay) => overlay,
            Err(error) => return error,
        };
        #[cfg(target_arch = "wasm32")]
        let _ = rhm_entry;
        #[cfg(target_arch = "wasm32")]
        return SBFILE_ERROR_NO_FILE;
        #[cfg(not(target_arch = "wasm32"))]
        {
            paths.push(OverlayRoot::Zip(Arc::new(overlay)));
            self.assets.invalidate_content(false);
        }
        #[cfg(not(target_arch = "wasm32"))]
        SBFILE_NO_ERROR
    }

    pub fn add_overlay_zip_bytes_for_mission(
        &self,
        mount_id: &str,
        bytes: Arc<[u8]>,
        rhm_entry: Option<&str>,
    ) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            tracing::warn!("ranked verifier filesystem rejected in-memory overlay {mount_id:?}");
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected in-memory overlay {mount_id:?}");
            return SBFILE_ERROR_READ;
        }
        if mount_id.trim().is_empty() {
            tracing::warn!("SbFileSystem::add_overlay_zip_bytes: empty mount id");
            return SBFILE_ERROR_NO_FILE;
        }
        let mut paths = self.overlay_paths.lock().unwrap();
        if paths
            .iter()
            .any(|candidate| candidate.display_path() == mount_id)
        {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        let overlay =
            match ZipOverlay::open_bytes_for_mission(mount_id.to_owned(), bytes, rhm_entry) {
                Ok(overlay) => overlay,
                Err(error) => return error,
            };
        {
            paths.push(OverlayRoot::Zip(Arc::new(overlay)));
            self.assets.invalidate_content(false);
        }
        SBFILE_NO_ERROR
    }

    pub fn remove_overlay(&self, path: &str) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            tracing::warn!("ranked verifier filesystem rejected overlay removal {path:?}");
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected overlay removal {path:?}");
            return SBFILE_ERROR_READ;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let requested = fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        #[cfg(target_arch = "wasm32")]
        let requested = PathBuf::from(path);
        let mut paths = self.overlay_paths.lock().unwrap();
        if let Some(index) = paths.iter().position(|candidate| {
            candidate.display_path() == path
                || candidate.display_path() == requested.to_string_lossy()
        }) {
            paths.remove(index);
            self.assets.invalidate_content(false);
            SBFILE_NO_ERROR
        } else {
            SBFILE_ERROR_PATH_NOT_IN_SET
        }
    }

    pub fn overlay_paths(&self) -> Vec<String> {
        self.overlay_paths
            .lock()
            .unwrap()
            .iter()
            .filter_map(|overlay| match overlay {
                OverlayRoot::Directory(path) => Some(path.to_string_lossy().into_owned()),
                OverlayRoot::Zip(_) => None,
            })
            .collect()
    }

    pub fn has_zip_overlays(&self) -> bool {
        self.overlay_paths
            .lock()
            .unwrap()
            .iter()
            .any(|overlay| matches!(overlay, OverlayRoot::Zip(_)))
    }

    pub fn set_primary_path(&self, path: &str) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            tracing::warn!("SbFileSystem::set_primary_path: ranked verifier root is locked");
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected primary path {path:?}");
            return SBFILE_ERROR_READ;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let path = match fs::canonicalize(path) {
            Ok(path) if path.is_dir() => path,
            Ok(path) => {
                tracing::warn!(
                    "SbFileSystem::set_primary_path: {} is not a directory",
                    path.display()
                );
                return SBFILE_ERROR_NO_FILE;
            }
            Err(error) => {
                tracing::warn!("SbFileSystem::set_primary_path: cannot open {path}: {error}");
                return SBFILE_ERROR_FILE_NOT_FOUND;
            }
        };
        #[cfg(target_arch = "wasm32")]
        let path = PathBuf::from(path);
        let mut primary = self.primary_path.lock().unwrap();
        *primary = Some(path);
        self.assets.invalidate_content(false);
        SBFILE_NO_ERROR
    }

    pub fn lock_ranked_verifier_primary_path(&self, path: &Path) -> i32 {
        self.lock_ranked_verifier_primary_path_inner(path, None)
    }

    pub fn lock_ranked_verifier_primary_path_with_locale(
        &self,
        path: &Path,
        resource_locale_root: &str,
    ) -> i32 {
        if resource_locale_root.is_empty()
            || resource_locale_root.starts_with('0')
            || resource_locale_root.len() > 8
            || !resource_locale_root
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            return SBFILE_ERROR_READ;
        }
        self.lock_ranked_verifier_primary_path_inner(path, Some(resource_locale_root))
    }

    fn lock_ranked_verifier_primary_path_inner(
        &self,
        path: &Path,
        resource_locale_root: Option<&str>,
    ) -> i32 {
        let canonical = match fs::canonicalize(path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => return SBFILE_ERROR_NO_FILE,
            Err(_) => return SBFILE_ERROR_FILE_NOT_FOUND,
        };
        if let Some(locale) = resource_locale_root {
            let locale_path = canonical.join(locale);
            let metadata = match fs::symlink_metadata(&locale_path) {
                Ok(metadata) => metadata,
                Err(_) => return SBFILE_ERROR_FILE_NOT_FOUND,
            };
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || ranked_locale_tree_is_ambiguous_or_unsafe(&locale_path)
            {
                return SBFILE_ERROR_READ;
            }
        }
        let mut locked = self.ranked_verifier_primary_path.lock().unwrap();
        if let Some(existing) = locked.as_ref() {
            let locale = self.locale_paths.lock().unwrap();
            return if existing == &canonical
                && locale.selected.as_deref() == resource_locale_root
                && locale.fallback.is_none()
            {
                SBFILE_NO_ERROR
            } else {
                SBFILE_ERROR_READ
            };
        }
        let locale_is_empty = {
            let locale = self.locale_paths.lock().unwrap();
            locale.selected.is_none() && locale.fallback.is_none() && locale.language.is_none()
        };
        if !self.overlay_paths.lock().unwrap().is_empty()
            || !self.alternate_paths.lock().unwrap().is_empty()
            || !locale_is_empty
        {
            tracing::warn!(
                "SbFileSystem::lock_ranked_verifier_primary_path: ambient lookup paths already exist"
            );
            return SBFILE_ERROR_READ;
        }
        *self.primary_path.lock().unwrap() = Some(canonical.clone());
        *self.locale_paths.lock().unwrap() = LocaleLookup {
            selected: resource_locale_root.map(str::to_owned),
            ..LocaleLookup::default()
        };
        *locked = Some(canonical);
        SBFILE_NO_ERROR
    }

    pub fn remove_alternate_path(&self, path: &str) -> i32 {
        if self.ranked_verifier_primary_path.lock().unwrap().is_some() {
            tracing::warn!("ranked verifier filesystem rejected alternate removal {path:?}");
            return SBFILE_ERROR_READ;
        }
        if self.official_projection_strict.load(Ordering::Acquire) {
            tracing::warn!("sealed filesystem rejected alternate removal {path:?}");
            return SBFILE_ERROR_READ;
        }
        let mut paths = self.alternate_paths.lock().unwrap();
        if let Some(index) = paths.iter().position(|candidate| candidate == path) {
            paths.remove(index);
            SBFILE_NO_ERROR
        } else {
            SBFILE_ERROR_PATH_NOT_IN_SET
        }
    }
}

/// A missing translation is an invalid language pack, not a reason to create
/// a mixed-language UI. Only recorded speech and cinematics are optional and
/// may fall back to the installed English pack.
fn is_optional_english_fallback_path(path: &str) -> bool {
    let normalized = path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("data/")
        .is_some_and(robin_util::asset_fs::is_optional_english_fallback_key)
}

fn is_required_locale_path(path: &str) -> bool {
    let normalized = path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("data/")
        .is_some_and(robin_util::asset_fs::is_required_locale_key)
}

fn is_locale_overlay_path(path: &str) -> bool {
    let normalized = path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("data/")
        .is_some_and(robin_util::asset_fs::is_locale_overlay_key)
}

fn normalise_locale_root(root: &str) -> Result<String, i32> {
    let normalised = root.replace('\\', "/");
    let normalised = normalised.trim_end_matches('/');
    if normalised.is_empty()
        || Path::new(normalised)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        tracing::warn!("SbFileSystem::set_locale_paths: invalid locale root {root:?}");
        return Err(SBFILE_ERROR_READ);
    }
    Ok(normalised.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn ranked_locale_tree_is_ambiguous_or_unsafe(root: &Path) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => return true,
        };
        let mut case_folded_names = std::collections::BTreeSet::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return true,
            };
            let name = match entry.file_name().into_string() {
                Ok(name) if !name.starts_with('.') => name,
                _ => return true,
            };
            if !case_folded_names.insert(name.to_ascii_lowercase()) {
                return true;
            }
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(_) => return true,
            };
            if metadata.file_type().is_symlink() {
                return true;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if !metadata.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(target_arch = "wasm32")]
fn ranked_locale_tree_is_ambiguous_or_unsafe(_root: &Path) -> bool {
    false
}

/// Read `path` from an overlay root, returning the bytes if present.
fn read_from_overlay(
    file_system: &SbFileSystem,
    root: &OverlayRoot,
    normalised: &str,
) -> Result<Option<AssetBytes>, i32> {
    match root {
        OverlayRoot::Directory(dir) => {
            let Some(resolved) = resolve_case_insensitive(&dir.join(normalised)) else {
                return Ok(None);
            };
            let resolved = fs::canonicalize(&resolved).map_err(|error| {
                tracing::warn!(
                    "overlay asset {} cannot be opened: {error}",
                    resolved.display()
                );
                SBFILE_ERROR_READ
            })?;
            if !resolved.starts_with(dir) {
                tracing::warn!(
                    "overlay asset {} escapes mount {}",
                    resolved.display(),
                    dir.display()
                );
                return Err(SBFILE_ERROR_READ);
            }
            try_read(file_system, &resolved.to_string_lossy())
        }
        OverlayRoot::Zip(z) => Ok(z.try_read(normalised).map(AssetBytes::from)),
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn prepared_snapshot_pins_working_directory_in_isolated_process() {
        const CHILD: &str = "ROBIN_DATA_IO_CWD_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sbfile::tests::prepared_snapshot_pins_working_directory_in_isolated_process",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        // Only this isolated test process changes CWD; other tests remain safe.
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for (root, bytes) in [(first.path(), b"first"), (second.path(), b"later")] {
            fs::create_dir(root.join("alt")).unwrap();
            fs::write(root.join("direct.bin"), bytes).unwrap();
            fs::write(root.join("alt/alternate.bin"), bytes).unwrap();
        }
        fs::write(second.path().join("new-only.bin"), b"later").unwrap();
        std::env::set_current_dir(first.path()).unwrap();
        let source = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        source.add_alternate_path("alt");
        let prepared = source.snapshot();
        std::env::set_current_dir(second.path()).unwrap();
        robin_util::asset_fs::global()
            .install_preloaded_asset("direct.bin", b"global-poison".to_vec())
            .unwrap();
        assert_eq!(prepared.read_all("direct.bin").unwrap(), b"first");
        assert_eq!(prepared.read_all("alternate.bin").unwrap(), b"first");
        assert_eq!(
            prepared.resolve_data_path("direct.bin").unwrap(),
            first.path().join("direct.bin")
        );
        assert_eq!(
            prepared.resolve_data_path("alternate.bin").unwrap(),
            first.path().join("alt/alternate.bin")
        );
        assert!(
            prepared
                .resolve_data_dir_layers("alt")
                .contains(&first.path().join("alt"))
        );
        assert!(
            !prepared
                .resolve_data_dir_layers("alt")
                .contains(&second.path().join("alt"))
        );
        assert!(!prepared.try_exists("new-only.bin").unwrap());
        assert!(prepared.read_all("new-only.bin").is_err());
        assert!(source.try_exists("new-only.bin").unwrap());
        assert_eq!(source.read_all("direct.bin").unwrap(), b"later");
        assert_eq!(
            prepared.snapshot().read_all("direct.bin").unwrap(),
            b"first"
        );
    }

    fn in_memory_zip(entries: &[(&str, &[u8])]) -> Arc<[u8]> {
        use std::io::Write as _;

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (path, bytes) in entries {
                writer.start_file(*path, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        Arc::from(cursor.into_inner())
    }

    #[test]
    fn in_memory_selected_mission_overlay_is_exact_and_newest_wins() {
        let shared_id = "memory://sbfile-test/shared.zip";
        let mission_id = "memory://sbfile-test/mission.zip";
        let path = "Data/Levels/BrowserMission.rhm";
        let _ = SbFile::remove_overlay(mission_id);
        let _ = SbFile::remove_overlay(shared_id);
        assert_eq!(
            SbFile::add_overlay_zip_bytes_for_mission(
                shared_id,
                in_memory_zip(&[(path, b"shared")]),
                Some(path),
            ),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::add_overlay_zip_bytes_for_mission(
                mission_id,
                in_memory_zip(&[(path, b"mission")]),
                Some(path),
            ),
            SBFILE_NO_ERROR
        );
        assert_eq!(SbFile::read_all(path).unwrap(), b"mission");
        assert_eq!(SbFile::remove_overlay(mission_id), SBFILE_NO_ERROR);
        assert_eq!(SbFile::read_all(path).unwrap(), b"shared");
        assert_eq!(SbFile::remove_overlay(shared_id), SBFILE_NO_ERROR);
    }

    #[test]
    fn official_projection_mount_is_one_way_and_has_no_host_fallback() {
        let source = tempfile::tempdir().unwrap();
        let core = tempfile::tempdir().unwrap();
        fs::create_dir_all(source.path().join("Data")).unwrap();
        fs::create_dir_all(core.path().join("Data")).unwrap();
        fs::write(source.path().join("Data/source.bin"), b"source").unwrap();
        fs::write(core.path().join("Data/core.bin"), b"core").unwrap();
        let source = fs::canonicalize(source.path()).unwrap();
        let core = fs::canonicalize(core.path()).unwrap();
        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));

        file_system
            .configure_official_projection_mounts(&source, "1033", &core)
            .unwrap();
        assert_eq!(file_system.read_all("Data/source.bin").unwrap(), b"source");
        assert_eq!(file_system.read_all("Data/core.bin").unwrap(), b"core");
        assert!(matches!(
            file_system.open(
                source.join("Data/source.bin").to_str().unwrap(),
                SB_FILE_READ
            ),
            Err(SBFILE_ERROR_READ)
        ));
        assert_eq!(
            file_system.add_alternate_path("host-fallback"),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.set_locale_paths(Some("1031"), None),
            SBFILE_ERROR_READ
        );
        assert!(
            file_system
                .configure_official_projection_mounts(&source, "1033", &core)
                .is_err()
        );
    }

    #[test]
    fn open_and_read() {
        let dir = std::env::temp_dir().join("sbfile_ro_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.bin");
        fs::write(&path, b"Hello").unwrap();
        let mut f = SbFile::open(path.to_str().unwrap(), SB_FILE_READ).unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(f.read(&mut buf), SBFILE_NO_ERROR);
        assert_eq!(&buf, b"Hello");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn deserialize_u32_le() {
        let dir = std::env::temp_dir().join("sbfile_ro_u32");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("u32.bin");
        fs::write(&path, [0xEF, 0xBE, 0xAD, 0xDE]).unwrap();
        let mut f = SbFile::open(path.to_str().unwrap(), SB_FILE_READ).unwrap();
        let mut v = 0u32;
        f.serialize_u32(&mut v).unwrap();
        assert_eq!(v, 0xDEADBEEF);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn deserialize_string() {
        let dir = std::env::temp_dir().join("sbfile_ro_str");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("str.bin");
        fs::write(&path, [0x05, 0x00, b'h', b'e', b'l', b'l', b'o']).unwrap();
        let mut f = SbFile::open(path.to_str().unwrap(), SB_FILE_READ).unwrap();
        let mut s = String::new();
        f.serialize_string(&mut s).unwrap();
        assert_eq!(s, "hello");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn checkpoint_valid() {
        let dir = std::env::temp_dir().join("sbfile_ro_chk");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("chk.bin");
        fs::write(&path, [0x77, 0x77]).unwrap();
        let mut f = SbFile::open(path.to_str().unwrap(), SB_FILE_READ).unwrap();
        f.checkpoint().unwrap();
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn alternate_paths() {
        let dir = std::env::temp_dir().join("sbfile_ro_alt");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("secret.dat"), b"x").unwrap();
        assert!(!SbFile::exists("secret.dat"));
        assert_eq!(
            SbFile::add_alternate_path(dir.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert!(SbFile::exists("secret.dat"));
        assert_eq!(
            SbFile::remove_alternate_path(dir.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_system_instances_isolate_paths_and_preserve_overlay_precedence() {
        let primary = tempfile::tempdir().unwrap();
        let overlay = tempfile::tempdir().unwrap();
        fs::write(primary.path().join("shared.dat"), b"primary").unwrap();
        fs::write(overlay.path().join("shared.dat"), b"overlay").unwrap();

        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mounted = SbFileSystem::new(assets.clone());
        mounted.set_primary_path(primary.path().to_str().unwrap());
        assert!(mounted.try_exists(".").unwrap());
        assert_eq!(
            mounted.add_overlay_path(overlay.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(mounted.read_all("shared.dat").unwrap(), b"overlay");

        let isolated = SbFileSystem::new(assets);
        isolated.set_primary_path(primary.path().to_str().unwrap());
        assert_eq!(isolated.read_all("shared.dat").unwrap(), b"primary");
        assert!(isolated.overlay_paths().is_empty());
    }

    #[test]
    fn prepared_snapshot_retains_mount_configuration_and_preloaded_bytes() {
        let root = tempfile::tempdir().unwrap();
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("Data/frozen.dat", b"before".to_vec())
            .unwrap();
        let source = SbFileSystem::new(assets.clone());
        let frozen = source.snapshot();
        let expected = frozen.mount_snapshot();
        assets
            .install_preloaded_asset("Data/frozen.dat", b"after".to_vec())
            .unwrap();
        assert_eq!(
            source.set_primary_path(root.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            source.set_locale_paths(Some("1033"), Some("2047")),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            source.add_overlay_path(root.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(frozen.mount_snapshot(), expected);
        assert_eq!(frozen.read_all("Data/frozen.dat").unwrap(), b"before");
        assert_eq!(source.read_all("Data/frozen.dat").unwrap(), b"after");
    }

    #[test]
    fn prepared_snapshot_retains_irreversible_verifier_confinement() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let source = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            source.lock_ranked_verifier_primary_path(root.path()),
            SBFILE_NO_ERROR
        );
        let frozen = source.snapshot();
        assert_eq!(
            frozen.set_primary_path(other.path().to_str().unwrap()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            frozen.set_locale_paths(Some("1033"), None),
            SBFILE_ERROR_READ
        );
    }

    #[test]
    fn ranked_verifier_root_is_irreversible_and_rejects_escape_paths() {
        let root = tempfile::tempdir().unwrap();
        let sibling = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Data/Text")).unwrap();
        fs::write(root.path().join("Data/Text/Level.res"), b"approved").unwrap();
        fs::write(sibling.path().join("secret.res"), b"outside").unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path(root.path()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.read_all("data/text/level.res").unwrap(),
            b"approved"
        );
        assert!(file_system.resolve_data_path("../secret.res").is_none());
        assert!(file_system.open("../secret.res", SB_FILE_READ).is_err());
        assert!(
            file_system
                .open(
                    sibling.path().join("secret.res").to_str().unwrap(),
                    SB_FILE_READ
                )
                .is_err()
        );

        assert_eq!(
            file_system.lock_ranked_verifier_primary_path(root.path()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path(sibling.path()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.set_primary_path(sibling.path().to_str().unwrap()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.add_alternate_path(sibling.path().to_str().unwrap()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.add_overlay_path(sibling.path().to_str().unwrap()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.add_overlay_zip(sibling.path().to_str().unwrap()),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.add_overlay_zip_for_mission(
                sibling.path().to_str().unwrap(),
                "Data/Levels/Custom.rhm",
            ),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.add_overlay_zip_bytes_for_mission(
                "memory://ranked-verifier/custom.zip",
                Arc::from([0_u8; 4]),
                Some("Data/Levels/Custom.rhm"),
            ),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.set_locale_paths(Some("Data/Locale"), None),
            SBFILE_ERROR_READ
        );
    }

    #[test]
    fn ranked_verifier_locale_mount_is_exact_and_case_insensitive_below_the_root() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("1033/data/Text")).unwrap();
        fs::write(
            root.path().join("1033/data/Text/Level.res"),
            b"approved locale",
        )
        .unwrap();
        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.read_all("Data/Text/Level.res").unwrap(),
            b"approved locale"
        );
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(root.path(), "2047"),
            SBFILE_ERROR_FILE_NOT_FOUND
        );
    }

    #[test]
    fn ranked_verifier_existing_selected_locale_never_falls_back_for_required_text() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("1033/Data/Text")).unwrap();
        fs::create_dir_all(root.path().join("2047/Data/Text")).unwrap();
        fs::create_dir_all(root.path().join("Data/Text")).unwrap();
        fs::write(
            root.path().join("2047/Data/Text/Level.res"),
            b"wrong locale",
        )
        .unwrap();
        fs::write(root.path().join("Data/Text/Level.res"), b"base fallback").unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_NO_ERROR
        );
        assert!(file_system.read_all("Data/Text/Level.res").is_err());
        assert!(
            file_system
                .resolve_data_path("Data/Text/Level.res")
                .is_none()
        );
        assert!(
            file_system
                .resolve_data_dir_layers("Data/Text")
                .iter()
                .all(|directory| directory.starts_with(root.path().join("1033")))
        );
    }

    #[test]
    fn ranked_verifier_resolves_normalized_sprite_bank_names_in_uppercase_data() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("1033")).unwrap();
        fs::create_dir_all(root.path().join("DATA")).unwrap();
        fs::write(root.path().join("DATA/robinhood.bks"), b"bank").unwrap();
        fs::write(root.path().join("DATA/robinhood.dic"), b"index").unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_NO_ERROR
        );
        assert_eq!(file_system.read_all("Data/robinhood.bks").unwrap(), b"bank");
        assert_eq!(
            file_system.read_all("Data/robinhood.dic").unwrap(),
            b"index"
        );
        assert!(
            file_system
                .resolve_data_path("./Data/robinhood.bks")
                .is_none()
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn ranked_verifier_locale_mount_rejects_case_ambiguous_or_missing_roots() {
        let missing = tempfile::tempdir().unwrap();
        fs::create_dir_all(missing.path().join("2047/Data/Text")).unwrap();
        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(missing.path(), "1033"),
            SBFILE_ERROR_FILE_NOT_FOUND
        );

        let ambiguous = tempfile::tempdir().unwrap();
        fs::create_dir_all(ambiguous.path().join("1033/Data/Text")).unwrap();
        fs::create_dir_all(ambiguous.path().join("1033/data/Text")).unwrap();
        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path_with_locale(ambiguous.path(), "1033"),
            SBFILE_ERROR_READ
        );
    }

    #[test]
    fn ranked_verifier_lock_rejects_preexisting_lookup_authority() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("1033")).unwrap();

        let alternate = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(alternate.add_alternate_path("ambient"), SBFILE_NO_ERROR);
        assert_eq!(
            alternate.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_ERROR_READ
        );

        let locale = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            locale.set_locale_paths(Some("ambient"), None),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            locale.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_ERROR_READ
        );

        let overlay_root = tempfile::tempdir().unwrap();
        let overlay = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            overlay.add_overlay_path(overlay_root.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            overlay.lock_ranked_verifier_primary_path_with_locale(root.path(), "1033"),
            SBFILE_ERROR_READ
        );
    }

    #[test]
    fn ranked_verifier_lock_requires_one_exact_numeric_locale_component() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("1033")).unwrap();

        for invalid in ["", "01033", "en-US", "1033/2047", "123456789"] {
            let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
            assert_eq!(
                file_system.lock_ranked_verifier_primary_path_with_locale(root.path(), invalid),
                SBFILE_ERROR_READ,
                "locale {invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn ranked_verifier_lookup_ignores_cwd_and_preloaded_vfs_assets() {
        let root = tempfile::tempdir().unwrap();
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("Data/ambient-vfs.dat", b"ambient".to_vec())
            .unwrap();
        let file_system = SbFileSystem::new(assets);
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path(root.path()),
            SBFILE_NO_ERROR
        );

        assert!(matches!(
            file_system.open("Cargo.toml", SB_FILE_READ),
            Err(SBFILE_ERROR_FILE_NOT_FOUND)
        ));
        assert!(!file_system.try_exists("Cargo.toml").unwrap());
        assert!(file_system.resolve_data_path("Cargo.toml").is_none());
        assert!(file_system.resolve_data_dir_layers("src").is_empty());
        assert!(matches!(
            file_system.open("Data/ambient-vfs.dat", SB_FILE_READ),
            Err(SBFILE_ERROR_FILE_NOT_FOUND)
        ));
        assert!(!file_system.try_exists("Data/ambient-vfs.dat").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn ranked_verifier_directory_resolution_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(outside.path().join("Data/Text")).unwrap();
        symlink(outside.path().join("Data"), root.path().join("Data")).unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.lock_ranked_verifier_primary_path(root.path()),
            SBFILE_NO_ERROR
        );
        assert!(file_system.resolve_data_dir_layers("Data/Text").is_empty());
        assert_eq!(file_system.try_exists("Data/Text"), Err(SBFILE_ERROR_READ));
    }

    fn write_layer_file(root: &Path, relative: &str, contents: &[u8]) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn locale_lookup_precedence_is_shared_by_all_resolution_apis() {
        let primary = tempfile::tempdir().unwrap();
        let overlay = tempfile::tempdir().unwrap();
        let relative = "Data/Sounds/Exclamations/locale-precedence.dat";

        let overlay_file = write_layer_file(overlay.path(), relative, b"overlay");
        let selected_file = write_layer_file(
            &primary.path().join("selected-locale"),
            relative,
            b"selected",
        );
        let fallback_file = write_layer_file(
            &primary.path().join("fallback-locale"),
            relative,
            b"fallback",
        );
        let primary_file = write_layer_file(primary.path(), relative, b"primary");
        let alternate_file =
            write_layer_file(&primary.path().join("ordinary"), relative, b"alternate");

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.set_primary_path(primary.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(file_system.add_alternate_path("ordinary"), SBFILE_NO_ERROR);
        assert_eq!(
            file_system.set_locale_paths(Some("selected-locale"), Some("fallback-locale")),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.add_overlay_path(overlay.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );

        let assert_resolves_to = |expected: &[u8], expected_path: &Path| {
            assert_eq!(
                file_system
                    .open(relative, SB_FILE_READ)
                    .unwrap()
                    .into_bytes(),
                expected
            );
            assert_eq!(file_system.read_all(relative).unwrap(), expected);
            assert!(file_system.try_exists(relative).unwrap());
            assert_eq!(
                file_system.resolve_data_path(relative).unwrap(),
                fs::canonicalize(expected_path).unwrap()
            );
        };

        assert_resolves_to(b"overlay", &overlay_file);
        assert_eq!(
            file_system.resolve_data_dir_layers("Data/Sounds/Exclamations"),
            vec![
                overlay.path().join("Data/Sounds/Exclamations"),
                primary
                    .path()
                    .join("selected-locale/Data/Sounds/Exclamations"),
                primary
                    .path()
                    .join("fallback-locale/Data/Sounds/Exclamations"),
                primary.path().join("Data/Sounds/Exclamations"),
                primary.path().join("ordinary/Data/Sounds/Exclamations"),
            ]
        );

        assert_eq!(
            file_system.remove_overlay(overlay.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_resolves_to(b"selected", &selected_file);
        fs::remove_file(selected_file).unwrap();
        assert_resolves_to(b"fallback", &fallback_file);
        fs::remove_file(fallback_file).unwrap();
        assert_resolves_to(b"primary", &primary_file);
        fs::remove_file(primary_file).unwrap();
        assert_resolves_to(b"alternate", &alternate_file);
    }

    #[test]
    fn english_fallback_is_not_used_for_text() {
        let primary = tempfile::tempdir().unwrap();
        write_layer_file(
            &primary.path().join("fallback-locale"),
            "Data/Text/only-in-english.res",
            b"english",
        );

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.set_primary_path(primary.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.set_locale_paths(Some("selected-locale"), Some("fallback-locale")),
            SBFILE_NO_ERROR
        );
        assert!(matches!(
            file_system.open("Data/Text/only-in-english.res", SB_FILE_READ),
            Err(SBFILE_ERROR_FILE_NOT_FOUND)
        ));
        assert!(file_system.resolve_data_dir_layers("Data/Text").is_empty());
    }

    #[test]
    fn locale_roots_cannot_override_simulation_inputs() {
        let primary = tempfile::tempdir().unwrap();
        write_layer_file(
            primary.path(),
            "Data/Configuration/profile.cpf",
            b"base-profile",
        );
        write_layer_file(
            &primary.path().join("selected-locale"),
            "Data/Configuration/profile.cpf",
            b"localized-profile",
        );

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.set_primary_path(primary.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.set_locale_paths(Some("selected-locale"), None),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system
                .open("Data/Configuration/profile.cpf", SB_FILE_READ)
                .unwrap()
                .into_bytes(),
            b"base-profile"
        );
    }

    #[test]
    fn presentation_language_is_owned_snapshotted_and_invalidates_localized_content() {
        let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let other = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            files.set_presentation_locale(None, None, Some("ja-JP")),
            SBFILE_NO_ERROR
        );
        let prepared = files.snapshot();
        let before = files.selection_snapshot();
        assert_eq!(
            files.set_presentation_locale(None, None, Some("en-US")),
            SBFILE_NO_ERROR
        );
        let after = files.selection_snapshot();
        assert_ne!(before.generation, after.generation);
        assert_eq!(before.content_generation, after.content_generation);
        assert_eq!(before.mission_generation, after.mission_generation);
        assert_eq!(prepared.presentation_locale().as_deref(), Some("ja-JP"));
        assert_eq!(files.presentation_locale().as_deref(), Some("en-US"));
        assert_eq!(other.presentation_locale(), None);
        assert_ne!(
            files.set_presentation_locale(Some("../escape"), None, Some("ru-RU")),
            SBFILE_NO_ERROR
        );
        assert_eq!(files.presentation_locale().as_deref(), Some("en-US"));
        assert_eq!(files.selection_snapshot().generation, after.generation);
        assert_eq!(files.set_locale_paths(None, None), SBFILE_NO_ERROR);
        assert_eq!(
            files.presentation_locale(),
            None,
            "raw mounts must not retain stale language policy"
        );
    }

    #[test]
    fn locale_updates_are_atomic_normalised_and_reject_invalid_roots() {
        let file_system = Arc::new(SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        assert_eq!(
            file_system.set_locale_paths(Some("de-DE/"), Some("1033\\")),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.locale_paths(),
            (Some("de-DE".to_string()), Some("1033".to_string()))
        );

        assert_eq!(
            file_system.set_locale_paths(Some("fr-FR"), Some("../escape")),
            SBFILE_ERROR_READ
        );
        assert_eq!(
            file_system.locale_paths(),
            (Some("de-DE".to_string()), Some("1033".to_string()))
        );

        let writer = Arc::clone(&file_system);
        let update = std::thread::spawn(move || {
            for _ in 0..2_000 {
                assert_eq!(
                    writer.set_locale_paths(Some("de-DE"), Some("en-US")),
                    SBFILE_NO_ERROR
                );
                assert_eq!(
                    writer.set_locale_paths(Some("fr-FR"), Some("en-GB")),
                    SBFILE_NO_ERROR
                );
            }
        });
        for _ in 0..2_000 {
            let pair = file_system.locale_paths();
            assert!(matches!(
                pair,
                (Some(ref selected), Some(ref fallback))
                    if (selected == "de-DE" && (fallback == "1033" || fallback == "en-US"))
                        || (selected == "fr-FR" && fallback == "en-GB")
            ));
        }
        update.join().unwrap();
    }

    #[test]
    fn duplicate_fallback_is_only_searched_once() {
        let root = tempfile::tempdir().unwrap();
        let locale_dir = root.path().join("EN-us/Data/Text");
        fs::create_dir_all(&locale_dir).unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.set_primary_path(root.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.set_locale_paths(Some("EN-us"), Some("en-US")),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.resolve_data_dir_layers("Data/Text"),
            vec![locale_dir]
        );
    }

    #[test]
    fn global_exists_uses_absolute_locale_roots() {
        let root = tempfile::tempdir().unwrap();
        let unique_name = format!("locale-global-{}.dat", fastrand::u64(..));
        let relative = format!("Data/Interface/{unique_name}");
        write_layer_file(root.path(), &relative, b"locale");

        assert_eq!(
            SbFile::set_locale_paths(Some(root.path().to_str().unwrap()), None),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::locale_paths(),
            (Some(root.path().to_string_lossy().into_owned()), None)
        );
        assert!(SbFile::exists(&relative));
        assert_eq!(SbFile::read_all(&relative).unwrap(), b"locale");
        assert_eq!(SbFile::set_locale_paths(None, None), SBFILE_NO_ERROR);
    }

    #[test]
    fn newest_overlay_wins_path_collisions() {
        let lower = tempfile::tempdir().unwrap();
        let upper = tempfile::tempdir().unwrap();
        fs::write(lower.path().join("shared.dat"), b"lower").unwrap();
        fs::write(upper.path().join("shared.dat"), b"upper").unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.add_overlay_path(lower.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.add_overlay_path(upper.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(file_system.read_all("shared.dat").unwrap(), b"upper");

        assert_eq!(
            file_system.remove_overlay(upper.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(file_system.read_all("shared.dat").unwrap(), b"lower");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn newest_zip_overlay_wins_shared_library_collisions() {
        let directory = tempfile::tempdir().unwrap();
        let shared = directory.path().join("shared.zip");
        let mission = directory.path().join("mission.zip");
        write_test_zip(
            &shared,
            &[("Data/Levels/lib/helper.lua", b"shared library")],
        );
        write_test_zip(
            &mission,
            &[("Data/Levels/lib/helper.lua", b"mission override")],
        );

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.add_overlay_zip(shared.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.add_overlay_zip(mission.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.read_all("Data/Levels/lib/helper.lua").unwrap(),
            b"mission override"
        );

        assert_eq!(
            file_system.remove_overlay(mission.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            file_system.read_all("Data/Levels/lib/helper.lua").unwrap(),
            b"shared library"
        );
    }

    #[test]
    fn file_system_reads_host_preloaded_assets_from_its_vfs() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset(
                "Data/Interface/UI/allied_portrait_background.png",
                b"png".to_vec(),
            )
            .unwrap();
        let file_system = SbFileSystem::new(assets);

        assert_eq!(
            file_system
                .read_all("Data/Interface/UI/allied_portrait_background.png")
                .unwrap(),
            b"png"
        );
    }

    #[test]
    fn shared_reads_retain_vfs_storage_and_ignore_the_stream_cursor() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let path = "Data/Interface/shared-read-fixture.bin";
        assets
            .install_preloaded_asset(path, b"shared bytes".to_vec())
            .unwrap();
        let source = assets.read_shared(path).unwrap();
        let file_system = SbFileSystem::new(assets.clone());
        let shared = file_system.read_shared(path).unwrap();
        assert_eq!(shared.as_ptr(), source.as_ptr());
        assert_eq!(file_system.read_all(path).unwrap(), source.as_ref());

        let mut stream = file_system.open(path, SB_FILE_READ).unwrap();
        let mut prefix = [0; 2];
        assert_eq!(stream.read(&mut prefix), SBFILE_NO_ERROR);
        assert_eq!(prefix, *b"sh");
        let backing = stream.into_shared_bytes();
        assert_eq!(backing.as_ref(), b"shared bytes");
        assert_eq!(backing.as_ptr(), source.as_ptr());

        assets
            .install_preloaded_asset(path, b"replacement".to_vec())
            .unwrap();
        assert_eq!(shared.as_ref(), b"shared bytes");
        assert_eq!(
            file_system.read_shared(path).unwrap().as_ref(),
            b"replacement"
        );
    }

    #[test]
    fn overlay_install_failure_is_not_reported_as_success() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let file_system = SbFileSystem::new(assets);
        let missing =
            std::env::temp_dir().join(format!("sbfile-missing-overlay-{}", fastrand::u64(..)));
        assert_eq!(
            file_system.add_overlay_path(missing.to_str().unwrap()),
            SBFILE_ERROR_FILE_NOT_FOUND
        );
        assert_eq!(
            file_system.set_primary_path(missing.to_str().unwrap()),
            SBFILE_ERROR_FILE_NOT_FOUND
        );
        assert!(file_system.overlay_paths().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn overlay_symlink_cannot_escape_mount() {
        use std::os::unix::fs::symlink;

        let overlay = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.dat"), b"secret").unwrap();
        symlink(outside.path(), overlay.path().join("escape")).unwrap();

        let file_system = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            file_system.add_overlay_path(overlay.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert!(matches!(
            file_system.open("escape/secret.dat", SB_FILE_READ),
            Err(SBFILE_ERROR_READ)
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let file = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn zip_overlay_layouts() {
        // Three zip layouts we care about:
        //   1. English-wrapped:  English/DATA/Levels/foo.rhm
        //   2. Locale-wrapped:   English/2047/Data/Text/Level.res
        //   3. Bare .rhm at root: foo.rhm
        //   4. Bare lib/:        lib/api.lua  -> Data/Levels/lib/api.lua
        let tmp = std::env::temp_dir().join("sbfile_zip_overlay");
        let _ = fs::create_dir_all(&tmp);

        let english_zip = tmp.join("english.zip");
        write_test_zip(
            &english_zip,
            &[
                ("English/DATA/Levels/foo.rhm", b"rhm-bytes"),
                ("English/2047/Data/Text/Level.res", b"res-bytes"),
            ],
        );

        let bare_zip = tmp.join("bare.zip");
        write_test_zip(&bare_zip, &[("S02_Lei_MP.rhm", b"vanilla-rhm")]);

        let lib_zip = tmp.join("lib.zip");
        write_test_zip(&lib_zip, &[("lib/api.lua", b"api-lua")]);

        // Mount + lookup.
        assert_eq!(
            SbFile::add_overlay_zip(english_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::add_overlay_zip(bare_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::add_overlay_zip(lib_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );

        // English-wrapped: addressable at normal datadir paths.
        assert!(SbFile::exists("Data/Levels/foo.rhm"));
        assert_eq!(
            SbFile::read_all("Data/Levels/foo.rhm").unwrap(),
            b"rhm-bytes"
        );
        assert!(SbFile::exists("2047/Data/Text/Level.res"));
        assert_eq!(
            SbFile::read_all("2047/Data/Text/Level.res").unwrap(),
            b"res-bytes"
        );
        // Case-insensitive lookup.
        assert_eq!(
            SbFile::read_all("DATA/LEVELS/foo.rhm").unwrap(),
            b"rhm-bytes"
        );

        // Bare .rhm: hoisted under Data/Levels/.
        assert_eq!(
            SbFile::read_all("Data/Levels/S02_Lei_MP.rhm").unwrap(),
            b"vanilla-rhm"
        );

        // lib/ folder: lands under Data/Levels/lib/.
        assert_eq!(
            SbFile::read_all("Data/Levels/lib/api.lua").unwrap(),
            b"api-lua"
        );

        // Clean up.
        assert_eq!(
            SbFile::remove_overlay(english_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::remove_overlay(bare_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            SbFile::remove_overlay(lib_zip.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn selected_mission_layout_accepts_borrowed_single_pass_entries() {
        let entries = std::collections::BTreeMap::from([
            ("French/DATA/Levels/map.rhm".to_owned(), 1),
            ("English/DATA/Levels/map.rhm".to_owned(), 2),
        ]);
        assert_eq!(
            detect_zip_layout_for_mission(entries.keys(), "ENGLISH/data/levels/MAP.RHM").unwrap(),
            ("english/".to_owned(), String::new())
        );
        assert_eq!(
            detect_zip_layout_for_mission(["wrapper\\map.rhm"].into_iter(), "wrapper/map.rhm")
                .unwrap(),
            ("wrapper/".to_owned(), "data/levels/".to_owned())
        );
        for selected in [
            "",
            "/map.rhm",
            "../map.rhm",
            "a//map.rhm",
            "a/./map.rhm",
            "a\\map.rhm",
            "map.txt",
        ] {
            let error = detect_zip_layout_for_mission([selected], selected).unwrap_err();
            assert!(
                error.contains("not a safe .rhm path"),
                "{selected}: {error}"
            );
        }
        assert!(
            detect_zip_layout_for_mission(entries.keys(), "missing.rhm")
                .unwrap_err()
                .contains("absent from the archive")
        );
        assert!(detect_zip_layout_for_mission(std::iter::empty::<&str>(), "map.rhm").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selected_mission_controls_language_overlay_root() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("languages.zip");
        write_test_zip(
            &archive,
            &[
                ("English/DATA/Levels/H06_Lin_VL.rhm", b"english-rhm"),
                ("English/2047/Data/Text/Level.res", b"english-text"),
                ("German/DATA/Levels/H06_Lin_VL.rhm", b"german-rhm"),
                ("German/1031/Data/Text/Level.res", b"german-text"),
                ("Polish/DATA/Levels/H06_Lin_VL.rhm", b"polish-rhm"),
            ],
        );

        let overlay =
            ZipOverlay::open_for_mission(&archive, Some("German/DATA/Levels/H06_Lin_VL.rhm"))
                .unwrap();
        assert_eq!(
            overlay.try_read("Data/Levels/H06_Lin_VL.rhm").unwrap(),
            b"german-rhm"
        );
        assert_eq!(
            overlay.try_read("1031/Data/Text/Level.res").unwrap(),
            b"german-text"
        );
        assert!(!overlay.exists("2047/Data/Text/Level.res"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn selected_folder_wrapped_mission_is_hoisted_to_levels() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("wrapped.zip");
        write_test_zip(
            &archive,
            &[
                ("H01_Lin_VL/H01_Lin_VL.rhm", b"mission-rhm"),
                ("H01_Lin_VL/H01_Lin_VL.lua", b"mission-lua"),
                ("H01_Lin_VL/enums.lua", b"mission-enums"),
            ],
        );

        let overlay =
            ZipOverlay::open_for_mission(&archive, Some("H01_Lin_VL/H01_Lin_VL.rhm")).unwrap();
        assert_eq!(
            overlay.try_read("Data/Levels/H01_Lin_VL.rhm").unwrap(),
            b"mission-rhm"
        );
        assert_eq!(
            overlay.try_read("Data/Levels/enums.lua").unwrap(),
            b"mission-enums"
        );
    }
}
