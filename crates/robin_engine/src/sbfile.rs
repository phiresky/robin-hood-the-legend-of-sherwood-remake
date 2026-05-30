//! Read-only filesystem abstraction for loading game data files.
//!
//! All Rust-side persistence uses serde (JSON). SbFile only reads
//! binary game data (`.cpf` profiles, level files, sprite data, etc.).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

pub const SBFILE_NO_ERROR: i32 = 0;
pub const SBFILE_ERROR_FILE_NOT_FOUND: i32 = -1;
pub const SBFILE_ERROR_NO_FILE: i32 = -4;
pub const SBFILE_ERROR_READ: i32 = -5;
pub const SBFILE_ERROR_SEEK: i32 = -7;
pub const SBFILE_ERROR_PATH_ALREADY_PRESENT: i32 = -10;
pub const SBFILE_ERROR_PATH_NOT_IN_SET: i32 = -11;
pub const SBFILE_ERROR_BAD_ARCHIVE: i32 = -20;

pub const SB_FILE_READ: i32 = 0x01;

static ALTERNATE_PATHS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static OVERLAY_PATHS: Mutex<Vec<OverlayRoot>> = Mutex::new(Vec::new());
static PRIMARY_PATH: Mutex<Option<String>> = Mutex::new(None);

/// One overlay root in the lookup stack.
///
/// `Directory` is a path on disk; lookups join it with the requested
/// path and consult the case-insensitive filesystem resolver.
/// `Zip` is a zip archive mounted in-memory (no extraction); lookups
/// consult a pre-built case-folded index built at mount time.
enum OverlayRoot {
    Directory(String),
    #[cfg(not(target_arch = "wasm32"))]
    Zip(Arc<ZipOverlay>),
}

impl OverlayRoot {
    fn display_path(&self) -> &str {
        match self {
            OverlayRoot::Directory(p) => p.as_str(),
            #[cfg(not(target_arch = "wasm32"))]
            OverlayRoot::Zip(z) => z.display_path.as_str(),
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
#[cfg(not(target_arch = "wasm32"))]
struct ZipOverlay {
    display_path: String,
    archive: Mutex<zip::ZipArchive<fs::File>>,
    /// Lower-cased + slash-normalized datadir path → zip entry index.
    index: HashMap<String, usize>,
}

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
    file: Cursor<Vec<u8>>,
    size: u64,
    position: u64,
    last_error: i32,
    version: u32,
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
pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_str()?;
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
    let normalised = path.replace('\\', "/");
    let p = Path::new(&normalised);

    // Overlay paths intentionally take precedence over the primary datadir.
    let overlay_paths = OVERLAY_PATHS.lock().unwrap();
    for overlay in overlay_paths.iter() {
        let OverlayRoot::Directory(dir) = overlay else {
            continue;
        };
        let full = format!("{}/{}", dir, normalised);
        if let Some(resolved) = resolve_case_insensitive(Path::new(&full))
            && resolved.is_file()
        {
            return Some(resolved);
        }
    }
    drop(overlay_paths);

    if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone() {
        let full = format!("{}/{}", primary, normalised);
        if let Some(resolved) = resolve_case_insensitive(Path::new(&full))
            && resolved.is_file()
        {
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
    let alt_paths = ALTERNATE_PATHS.lock().unwrap();
    for alt in alt_paths.iter() {
        if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone() {
            let full = format!("{}/{}/{}", primary, alt, normalised);
            if let Some(resolved) = resolve_case_insensitive(Path::new(&full))
                && resolved.is_file()
            {
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
fn try_read(path: &str) -> Option<Vec<u8>> {
    match robin_util::asset_fs::read(path) {
        Ok(bytes) => return Some(bytes),
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => {
            tracing::trace!("asset {path}: not found");
        }
        Err(e) => tracing::warn!("asset read failed for {path}: {e}"),
    }
    if let Some(resolved) = resolve_case_insensitive(Path::new(path)) {
        match robin_util::asset_fs::read(&resolved) {
            Ok(bytes) => return Some(bytes),
            Err(robin_util::asset_fs::AssetError::NotFound(_)) => {
                tracing::trace!(
                    "asset {} (case-resolved from {path}): not found",
                    resolved.display()
                );
            }
            Err(e) => tracing::warn!(
                "asset read failed for {} (case-resolved from {path}): {e}",
                resolved.display()
            ),
        }
    }
    None
}

impl SbFile {
    pub fn open(path: &str, _flags: i32) -> Result<Self, i32> {
        let normalised = path.replace('\\', "/");
        let overlay_paths = OVERLAY_PATHS.lock().unwrap();
        for overlay in overlay_paths.iter() {
            if let Some(bytes) = read_from_overlay(overlay, &normalised) {
                return Ok(Self::from_bytes(bytes));
            }
        }
        drop(overlay_paths);
        if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone()
            && let Some(bytes) = try_read(&format!("{primary}/{normalised}"))
        {
            return Ok(Self::from_bytes(bytes));
        }
        if let Some(bytes) = try_read(&normalised) {
            return Ok(Self::from_bytes(bytes));
        }
        let alt_paths = ALTERNATE_PATHS.lock().unwrap();
        for alt in alt_paths.iter() {
            if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone()
                && let Some(bytes) = try_read(&format!("{primary}/{alt}/{normalised}"))
            {
                return Ok(Self::from_bytes(bytes));
            }
            if let Some(bytes) = try_read(&format!("{alt}/{normalised}")) {
                return Ok(Self::from_bytes(bytes));
            }
        }
        tracing::warn!(
            "SbFile::open: {normalised} not found (tried direct + {} alternate paths)",
            alt_paths.len()
        );
        Err(SBFILE_ERROR_FILE_NOT_FOUND)
    }

    fn from_bytes(bytes: Vec<u8>) -> Self {
        let size = bytes.len() as u64;
        SbFile {
            file: Cursor::new(bytes),
            size,
            position: 0,
            last_error: SBFILE_NO_ERROR,
            version: 0,
        }
    }

    pub fn read_all(path: &str) -> Result<Vec<u8>, i32> {
        let mut file = Self::open(path, SB_FILE_READ)?;
        let mut bytes = vec![0; file.get_size() as usize];
        file.serialize_bytes(&mut bytes)?;
        Ok(bytes)
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

    /// Clears `line`, then loops reading one byte at a time, appending
    /// it unless the byte is `\n` or `\r`, until a `\n` is consumed or
    /// EOF is reached. Returns `!self.is_eof()` — i.e. true if the file
    /// may still have more data, false if EOF was reached (whether
    /// mid-line or just past the terminating newline).

    pub fn exists(path: &str) -> bool {
        let n = path.replace('\\', "/");
        let overlays = OVERLAY_PATHS.lock().unwrap();
        for overlay in overlays.iter() {
            if overlay_root_exists(overlay, &n) {
                return true;
            }
        }
        drop(overlays);
        if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone() {
            let c = format!("{}/{}", primary, n);
            if Path::new(&c).exists() || resolve_case_insensitive(Path::new(&c)).is_some() {
                return true;
            }
        }
        if robin_util::asset_fs::exists(&n) {
            return true;
        }
        let p = Path::new(&n);
        if p.exists() {
            return true;
        }
        if resolve_case_insensitive(p).is_some() {
            return true;
        }
        let alts = ALTERNATE_PATHS.lock().unwrap();
        for alt in alts.iter() {
            if let Some(primary) = PRIMARY_PATH.lock().unwrap().clone() {
                let c = format!("{}/{}/{}", primary, alt, n);
                if Path::new(&c).exists() || resolve_case_insensitive(Path::new(&c)).is_some() {
                    return true;
                }
            }
            let c = format!("{}/{}", alt, n);
            if Path::new(&c).exists() || resolve_case_insensitive(Path::new(&c)).is_some() {
                return true;
            }
        }
        false
    }

    pub fn add_alternate_path(path: &str) -> i32 {
        let mut p = ALTERNATE_PATHS.lock().unwrap();
        if p.iter().any(|x| x == path) {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        p.push(path.to_string());
        SBFILE_NO_ERROR
    }

    pub fn add_overlay_path(path: &str) -> i32 {
        let mut p = OVERLAY_PATHS.lock().unwrap();
        if p.iter().any(|x| x.display_path() == path) {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        p.push(OverlayRoot::Directory(path.to_string()));
        SBFILE_NO_ERROR
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
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_overlay_zip(zip_path: &str) -> i32 {
        let mut p = OVERLAY_PATHS.lock().unwrap();
        if p.iter().any(|x| x.display_path() == zip_path) {
            return SBFILE_ERROR_PATH_ALREADY_PRESENT;
        }
        let overlay = match ZipOverlay::open(Path::new(zip_path)) {
            Ok(o) => o,
            Err(e) => return e,
        };
        p.push(OverlayRoot::Zip(Arc::new(overlay)));
        SBFILE_NO_ERROR
    }

    /// Remove an overlay by its registered path (works for both directory
    /// and zip overlays).  Returns `SBFILE_ERROR_PATH_NOT_IN_SET` if not
    /// found.
    pub fn remove_overlay(path: &str) -> i32 {
        let mut p = OVERLAY_PATHS.lock().unwrap();
        if let Some(i) = p.iter().position(|x| x.display_path() == path) {
            p.remove(i);
            SBFILE_NO_ERROR
        } else {
            SBFILE_ERROR_PATH_NOT_IN_SET
        }
    }

    /// Returns all directory-overlay paths (in priority order).  Zip
    /// overlays are intentionally excluded: this API exists for callers
    /// that want to walk the directory tree (e.g. enumerate
    /// `Data/Characters/*.rhs.d/`), which doesn't apply to in-memory zip
    /// roots.
    pub fn overlay_paths() -> Vec<String> {
        OVERLAY_PATHS
            .lock()
            .unwrap()
            .iter()
            .filter_map(|o| match o {
                OverlayRoot::Directory(p) => Some(p.clone()),
                #[cfg(not(target_arch = "wasm32"))]
                OverlayRoot::Zip(_) => None,
            })
            .collect()
    }

    pub fn set_primary_path(path: &str) -> i32 {
        *PRIMARY_PATH.lock().unwrap() = Some(path.to_string());
        SBFILE_NO_ERROR
    }

    pub fn remove_alternate_path(path: &str) -> i32 {
        let mut p = ALTERNATE_PATHS.lock().unwrap();
        if let Some(i) = p.iter().position(|x| x == path) {
            p.remove(i);
            SBFILE_NO_ERROR
        } else {
            SBFILE_ERROR_PATH_NOT_IN_SET
        }
    }
}

/// Read `path` from an overlay root, returning the bytes if present.
fn read_from_overlay(root: &OverlayRoot, normalised: &str) -> Option<Vec<u8>> {
    match root {
        OverlayRoot::Directory(dir) => try_read(&format!("{dir}/{normalised}")),
        #[cfg(not(target_arch = "wasm32"))]
        OverlayRoot::Zip(z) => z.try_read(normalised),
    }
}

/// Test whether `path` exists in an overlay root.
fn overlay_root_exists(root: &OverlayRoot, normalised: &str) -> bool {
    match root {
        OverlayRoot::Directory(dir) => {
            let c = format!("{dir}/{normalised}");
            Path::new(&c).exists() || resolve_case_insensitive(Path::new(&c)).is_some()
        }
        #[cfg(not(target_arch = "wasm32"))]
        OverlayRoot::Zip(z) => z.exists(normalised),
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
