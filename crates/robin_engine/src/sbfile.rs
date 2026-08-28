//! Read-only filesystem abstraction for loading game data files.
//!
//! All Rust-side persistence uses serde (JSON). SbFile only reads
//! binary game data (`.cpf` profiles, level files, sprite data, etc.).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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

/// Instance-owned SBFile lookup state.
///
/// The original engine stored alternate paths in a static vector
/// (`original-code/sblibng/SBFile.cpp:23`) and searched it in insertion order
/// after the requested path (`SBFile.cpp:1014-1042`). This type preserves that
/// ordering while allowing independent instances for tools and tests.
pub struct SbFileSystem {
    assets: Arc<robin_util::asset_fs::AssetVfs>,
    alternate_paths: Mutex<Vec<String>>,
    overlay_paths: Mutex<Vec<OverlayRoot>>,
    primary_path: Mutex<Option<PathBuf>>,
}

impl SbFileSystem {
    pub fn new(assets: Arc<robin_util::asset_fs::AssetVfs>) -> Self {
        Self {
            assets,
            alternate_paths: Mutex::new(Vec::new()),
            overlay_paths: Mutex::new(Vec::new()),
            primary_path: Mutex::new(None),
        }
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
enum OverlayRoot {
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
    archive: Mutex<zip::ZipArchive<fs::File>>,
    /// Lower-cased + slash-normalized datadir path → zip entry index.
    index: HashMap<String, usize>,
}

impl ZipOverlay {
    /// Open a zip file, detect its layout, and build the index.
    fn open(path: &Path) -> Result<Self, i32> {
        let file = fs::File::open(path).map_err(|e| {
            tracing::warn!("ZipOverlay::open: failed to open {}: {e}", path.display());
            SBFILE_ERROR_FILE_NOT_FOUND
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            tracing::warn!("ZipOverlay::open: not a valid zip {}: {e}", path.display());
            SBFILE_ERROR_BAD_ARCHIVE
        })?;

        let mut entry_names: Vec<String> = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|_| SBFILE_ERROR_BAD_ARCHIVE)?;
            if entry.is_dir() {
                entry_names.push(String::new()); // placeholder, never indexed
                continue;
            }
            entry_names.push(entry.name().replace('\\', "/"));
        }

        let (strip, prepend) = detect_zip_layout(&entry_names);
        tracing::info!(
            "ZipOverlay::open: {} (strip={:?}, prepend={:?}, entries={})",
            path.display(),
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
            display_path: path.to_string_lossy().into_owned(),
            archive: Mutex::new(archive),
            index,
        })
    }

    fn try_read(&self, path: &str) -> Option<Vec<u8>> {
        let key = path.replace('\\', "/").to_ascii_lowercase();
        let idx = *self.index.get(&key)?;
        let mut archive = self.archive.lock().unwrap();
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("ZipOverlay::try_read: zip entry {idx} read failed: {e}");
                return None;
            }
        };
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        if let Err(e) = entry.read_to_end(&mut bytes) {
            tracing::warn!("ZipOverlay::try_read: zip entry {idx} read failed: {e}");
            return None;
        }
        Some(bytes)
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
/// Tries the path directly (case-insensitive), then each registered alternate
/// path.  Returns `None` if the file cannot be found anywhere.  Used by the
/// video player to obtain a real path for ffmpeg to open.
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
        let mut candidates: Vec<PathBuf> = Vec::new();
        {
            let overlays = self.overlay_paths.lock().unwrap();
            for overlay in overlays.iter() {
                #[allow(irrefutable_let_patterns)] // wasm has no Zip variant
                if let OverlayRoot::Directory(dir) = overlay {
                    candidates.push(dir.join(&normalised));
                }
            }
        }
        let primary = self.primary_path.lock().unwrap().clone();
        if let Some(primary) = &primary {
            candidates.push(primary.join(&normalised));
        }
        candidates.push(PathBuf::from(&normalised));
        for alt in self.alternate_paths.lock().unwrap().iter() {
            if let Some(primary) = &primary {
                candidates.push(primary.join(alt).join(&normalised));
            }
            candidates.push(Path::new(alt).join(&normalised));
        }
        candidates
            .into_iter()
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
        if !p.is_absolute()
            && p.components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            tracing::warn!("resolve_data_path: rejected escaping path {normalised}");
            return None;
        }

        // Overlay paths intentionally take precedence over the primary datadir.
        let overlay_paths = self.overlay_paths.lock().unwrap();
        for overlay in overlay_paths.iter() {
            let OverlayRoot::Directory(dir) = overlay else {
                continue;
            };
            let full = dir.join(&normalised);
            if let Some(resolved) = resolve_contained_file(dir, &full) {
                return Some(resolved);
            }
        }
        drop(overlay_paths);

        if let Some(primary) = self.primary_path.lock().unwrap().clone() {
            let full = primary.join(&normalised);
            if let Some(resolved) = resolve_contained_file(&primary, &full) {
                return Some(resolved);
            }
        }

        // Direct path
        if let Some(resolved) = resolve_case_insensitive(p)
            && resolved.is_file()
        {
            return Some(resolved);
        }

        // Alternate paths
        let alt_paths = self.alternate_paths.lock().unwrap();
        for alt in alt_paths.iter() {
            if let Some(primary) = self.primary_path.lock().unwrap().clone() {
                let full = primary.join(alt).join(&normalised);
                if let Some(resolved) = resolve_contained_file(&primary, &full) {
                    return Some(resolved);
                }
            }
            let full = format!("{}/{}", alt, normalised);
            if let Some(resolved) = resolve_case_insensitive(Path::new(&full))
                && resolved.is_file()
            {
                return Some(resolved);
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
    if let Some(resolved) = resolve_case_insensitive(Path::new(path)) {
        match robin_util::asset_fs::read_shared(&resolved) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(robin_util::asset_fs::AssetError::NotFound(_)) => {
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
        if requested.is_absolute() {
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
        for overlay in overlay_paths.iter() {
            if let Some(bytes) = read_from_overlay(self, overlay, &normalised)? {
                return Ok(SbFile::from_bytes(bytes, normalised.clone()));
            }
        }
        drop(overlay_paths);
        if let Some(primary) = self.primary_path.lock().unwrap().clone()
            && let Some(bytes) = try_read(self, &primary.join(&normalised).to_string_lossy())?
        {
            return Ok(SbFile::from_bytes(bytes, normalised.clone()));
        }
        if let Some(bytes) = try_read(self, &normalised)? {
            return Ok(SbFile::from_bytes(bytes, normalised.clone()));
        }
        let alt_paths = self.alternate_paths.lock().unwrap();
        for alt in alt_paths.iter() {
            if let Some(primary) = self.primary_path.lock().unwrap().clone()
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
            "SbFile::open: {normalised} not found (tried direct + {} alternate paths)",
            alt_paths.len()
        );
        Err(SBFILE_ERROR_FILE_NOT_FOUND)
    }

    pub fn read_all(&self, path: &str) -> Result<Vec<u8>, i32> {
        Ok(self.open(path, SB_FILE_READ)?.into_bytes())
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

    /// Consume the stream and return its full backing buffer.
    ///
    /// Open buffers the entire file up front, so callers that want all
    /// the bytes can take the buffer directly instead of copying it
    /// back out through the stream API — for the sprite bank that copy
    /// is hundreds of megabytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.file.into_inner().into_vec()
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

    pub fn set_primary_path(path: &str) -> i32 {
        global_file_system().set_primary_path(path)
    }

    /// Resolve a relative datadir *directory* to every existing native
    /// directory across the search order (overlays, primary, direct,
    /// then alternates such as the language folder), case-insensitively.
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
    pub fn try_exists(&self, path: &str) -> Result<bool, i32> {
        let normalised = path.replace('\\', "/");
        let requested = Path::new(&normalised);
        if !requested.is_absolute()
            && requested
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(SBFILE_ERROR_READ);
        }

        if !requested.is_absolute() {
            let overlays = self.overlay_paths.lock().unwrap();
            for overlay in overlays.iter() {
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

        if !requested.is_absolute()
            && let Some(primary) = self.primary_path.lock().unwrap().clone()
            && path_exists_contained(&primary, &primary.join(&normalised))?
        {
            return Ok(true);
        }
        match self.assets.try_exists(requested) {
            Ok(true) => return Ok(true),
            Ok(false) | Err(robin_util::asset_fs::AssetError::InvalidPath(_)) => {}
            Err(error) => {
                tracing::warn!("SbFileSystem::try_exists({normalised}): {error}");
                return Err(SBFILE_ERROR_READ);
            }
        }
        if resolve_case_insensitive(requested).is_some() {
            return Ok(true);
        }

        if !requested.is_absolute() {
            let alternate_paths = self.alternate_paths.lock().unwrap();
            for alternate in alternate_paths.iter() {
                if let Some(primary) = self.primary_path.lock().unwrap().clone()
                    && path_exists_contained(&primary, &primary.join(alternate).join(&normalised))?
                {
                    return Ok(true);
                }
                if resolve_case_insensitive(&Path::new(alternate).join(&normalised)).is_some() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn add_alternate_path(&self, path: &str) -> i32 {
        let mut paths = self.alternate_paths.lock().unwrap();
        if paths.iter().any(|candidate| candidate == path) {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        paths.push(path.to_string());
        SBFILE_NO_ERROR
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_overlay_path(&self, path: &str) -> i32 {
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
        let mut paths = self.overlay_paths.lock().unwrap();
        if paths
            .iter()
            .any(|candidate| candidate.display_path() == zip_path)
        {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        let overlay = match ZipOverlay::open(Path::new(zip_path)) {
            Ok(overlay) => overlay,
            Err(error) => return error,
        };
        paths.push(OverlayRoot::Zip(Arc::new(overlay)));
        SBFILE_NO_ERROR
    }

    pub fn remove_overlay(&self, path: &str) -> i32 {
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

    pub fn set_primary_path(&self, path: &str) -> i32 {
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
        *self.primary_path.lock().unwrap() = Some(path);
        SBFILE_NO_ERROR
    }

    pub fn remove_alternate_path(&self, path: &str) -> i32 {
        let mut paths = self.alternate_paths.lock().unwrap();
        if let Some(index) = paths.iter().position(|candidate| candidate == path) {
            paths.remove(index);
            SBFILE_NO_ERROR
        } else {
            SBFILE_ERROR_PATH_NOT_IN_SET
        }
    }
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
}
