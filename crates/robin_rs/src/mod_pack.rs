//! Custom-mission pack metadata and mount machinery.
//!
//! Mirrors the `details.json` files written next to each mod zip under
//! `datadirs/mods/<slug>/details.json` (produced by the rhmods.com
//! scraper) and adds:
//!
//! - scanning the mods dir into [`DiscoveredMod`] entries
//! - enumerating missions inside each mod's version zips
//! - peeking each `.rhm` to recover its map (proto-level) filename
//!   without loading the whole level
//! - mounting a chosen mod's zip as a non-destructive overlay through
//!   an explicit `SbFileSystem`, with the Spellforge `lib/` zip layered
//!   underneath when needed
//!
//! Lua / Spellforge runtime support is the job of a separate agent —
//! this module is purely concerned with discovery and file-system layering.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use robin_engine::sbfile::{SBFILE_NO_ERROR, SbFileSystem, detect_zip_layout_for_mission};

/// Top-level metadata for one custom mission mod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetails {
    pub slug: String,
    pub title: String,
    pub page_url: String,
    pub author: String,
    /// SPDX expression, licence name, or other author-supplied
    /// redistribution permission displayed before multiplayer hosting.
    #[serde(default)]
    pub license: String,
    pub map: String,
    /// Free-form date string as displayed on rhmods.com (e.g. `"Feb 12, 2026"`).
    pub uploaded: String,
    /// `"Vanilla"` and/or `"Spellforge"`.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub likes: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub versions: Vec<ModVersion>,
    /// Hackable JSON levels shipped as an
    /// always-mounted overlay datadir (see [`crate::main_entry`]'s
    /// `MODS_DIR`), stored as a directory or ZIP. Each value is the
    /// `<mission>` of a `Data/Levels/<mission>.level.json` descriptor.
    /// Such mods need no `versions` — the picker launches them through
    /// the hackable-level path instead of mounting a zip.
    #[serde(default)]
    pub hackable_missions: Vec<String>,
}

/// One uploaded version of the mod. Each version is mirrored as a separate
/// zip next to the `details.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModVersion {
    pub date_uploaded: String,
    #[serde(default)]
    pub version_notes: String,
    pub download_url: String,
    /// Filename of the mirrored zip, relative to the mod's directory.
    pub local_file: String,
}

impl ModDetails {
    pub fn requires_spellforge(&self) -> bool {
        self.tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case("Spellforge"))
    }

    pub fn load(path: &Path) -> Result<Self, ModDetailsError> {
        let bytes = fs::read(path).map_err(|e| ModDetailsError::Io(path.to_path_buf(), e))?;
        serde_json::from_slice(&bytes).map_err(|e| ModDetailsError::Parse(path.to_path_buf(), e))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModDetailsError {
    #[error("reading {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("parsing {0}: {1}")]
    Parse(PathBuf, #[source] serde_json::Error),
}

// ── Discovery ───────────────────────────────────────────────────

/// A mod discovered on disk: its parsed `details.json` and containing root.
#[derive(Debug, Clone)]
pub struct DiscoveredMod {
    pub details: ModDetails,
    /// Directory or standalone ZIP containing the mod root.
    pub mod_dir: PathBuf,
}

/// Mount a mod root with the same logical paths for directories and ZIPs.
pub fn mount_mod_overlay(files: &SbFileSystem, path: &Path) -> i32 {
    if path.is_dir() {
        files.add_overlay_path(&path.to_string_lossy())
    } else {
        match fs::canonicalize(path) {
            Ok(path) => files.add_overlay_zip(&path.to_string_lossy()),
            Err(error) => {
                tracing::warn!("Cannot open mod archive {}: {error}", path.display());
                robin_engine::sbfile::SBFILE_ERROR_NO_FILE
            }
        }
    }
}

impl DiscoveredMod {
    /// Resolve `version.local_file` to an absolute path on disk.
    pub fn version_zip_path(&self, version: &ModVersion) -> PathBuf {
        self.mod_dir.join(&version.local_file)
    }

    /// Locally-cached preview image, if one was placed next to the
    /// `details.json` by an external tool.  The picker shows it in the
    /// detail pane; absence is not an error.  Convention: a single PNG
    /// named `preview.png` (kept simple — the picker doesn't need a
    /// gallery and online fetches are out of scope).
    pub fn preview_image_path(&self) -> Option<PathBuf> {
        let p = self.mod_dir.join("preview.png");
        p.is_file().then_some(p)
    }
}

/// Scan `mods_root` for directories and ZIPs containing `details.json`, returning all
/// successfully-parsed mods.  Parse failures are logged at `warn` and
/// skipped — a bad single `details.json` shouldn't make the entire
/// picker unavailable.
pub fn scan_mods_dir(mods_root: &Path) -> Vec<DiscoveredMod> {
    let mut out = Vec::new();
    for path in discovery_paths(mods_root) {
        if !path.is_dir()
            && !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        {
            continue;
        }
        let files = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let status = mount_mod_overlay(&files, &path);
        if status != SBFILE_NO_ERROR {
            tracing::warn!("scan_mods_dir: cannot mount {}: {status}", path.display());
            continue;
        }
        let source = files
            .overlay_sources()
            .into_iter()
            .next()
            .expect("successful mod mount");
        let bytes = match files.read_overlay(&source, "details.json") {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!("scan_mods_dir: cannot read {source}/details.json: {error}");
                continue;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(details) => out.push(DiscoveredMod {
                details,
                mod_dir: path,
            }),
            Err(e) => {
                tracing::warn!("scan_mods_dir: skipping {source}/details.json: {e}");
            }
        }
    }
    out.sort_by(|a, b| a.details.title.cmp(&b.details.title));
    out
}

/// Missing optional roots are ordinary discovery misses. Other failures remain
/// best-effort too, but must not hide the reason installed content was omitted.
fn discovery_paths(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let entries = match fs::read_dir(root) {
        Ok(entries) => Some(entries),
        Err(error) => {
            if error.kind() == std::io::ErrorKind::NotFound {
                tracing::info!(path = %root.display(), "mod discovery directory is absent");
            } else {
                tracing::warn!(path = %root.display(), %error, "cannot read mod discovery directory");
            }
            None
        }
    };
    entries.into_iter().flatten().filter_map(move |entry| match entry {
        Ok(entry) => Some(entry.path()),
        Err(error) => {
            tracing::warn!(path = %root.display(), %error, "skipping unreadable mod discovery entry");
            None
        }
    })
}

/// Combine installed and bundled discovery without scanning an identical root
/// twice. Keep distinct sources even when their metadata matches: launchers need
/// the original directory to resolve the selected archive. Equal titles retain
/// configured-root precedence through the stable sort.
pub(crate) fn scan_mission_roots(configured: &Path, bundled: Option<&Path>) -> Vec<DiscoveredMod> {
    let mut mods = scan_mods_dir(configured);
    if let Some(bundled) = bundled.filter(|root| *root != configured) {
        mods.extend(scan_mods_dir(bundled));
        mods.sort_by(|a, b| a.details.title.cmp(&b.details.title));
    }
    mods
}

/// Resolve the directory the mod scanner should walk.
///
/// Priority order:
/// 1. `ROBINHOOD_MODS_DIR` environment variable, if set.
/// 2. `<primary_datadir>/../mods/`, if `ROBINHOOD_DATA_DIR` is set.
/// 3. `./datadirs/mods/` relative to the process working directory
///    (the repo root layout used in development).
///
/// Always returns *some* path even if it doesn't exist on disk —
/// `scan_mods_dir` already handles missing directories by logging and
/// returning an empty vec.
pub fn default_mods_root() -> PathBuf {
    if let Ok(dir) = std::env::var("ROBINHOOD_MODS_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(data_dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        let primary = PathBuf::from(data_dir);
        if let Some(parent) = primary.parent() {
            return parent.join("mods");
        }
    }
    PathBuf::from("datadirs/mods")
}

// ── Mission enumeration ─────────────────────────────────────────

/// One launchable row in the custom-mission picker: a triple of
/// `(mod, version, .rhm file inside the version's zip)`.  Mods that
/// bundle multiple `.rhm` files (e.g. `meet-the-spy` ships both
/// `CR02_Yrk_VL.rhm` and `H06_Lin_VL.rhm`) expand into one
/// `MissionEntry` per file; multi-language version zips
/// (`v1.2-EN.zip`, `v1.2-DE.zip`) expand into one per version.
#[derive(Debug, Clone)]
pub struct MissionEntry {
    pub mod_slug: String,
    pub mod_title: String,
    pub author: String,
    pub source_url: String,
    pub license: String,
    pub description: String,
    pub map: String,
    pub requires_spellforge: bool,

    /// Display label for the version — `version_notes` if non-empty,
    /// otherwise `date_uploaded`.  Distinguishes EN/DE variants in the UI.
    pub version_label: String,
    pub version_zip: PathBuf,

    /// `.rhm` filename inside the zip (e.g. `S02_Lei_MP.rhm`,
    /// `English/DATA/Levels/CR02_Yrk_VL.rhm`).  Stored exactly as the
    /// zip entry names it so re-opening for the header peek doesn't
    /// have to re-detect the layout.
    pub rhm_zip_entry: String,

    /// Bare basename (no extension) — what the engine uses to address
    /// the mission via `Data/Levels/<basename>.rhm`.  For hackable rows
    /// this is the hackable mission filename instead.
    pub rhm_basename: String,

    /// True for hackable JSON levels (`ModDetails::hackable_missions`):
    /// launched through the hackable-level path with no zip mount.
    pub hackable: bool,

    /// Resolved status: `Ok` rows are launchable; `Broken` rows are
    /// shown greyed-out with the reason as a tooltip / status string.
    pub status: MissionStatus,

    /// Locally-cached preview image path (`preview.png` next to
    /// `details.json`), if any.
    pub preview_image: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum MissionStatus {
    /// Header peek succeeded; carries the proto-level (.rhp) filename
    /// the engine should pair with this `.rhm`.
    Ok {
        map_filename: String,
    },
    Broken {
        reason: String,
    },
}

impl MissionStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, MissionStatus::Ok { .. })
    }
}

/// Expand discovered mods into one [`MissionEntry`] per
/// `(mod, version, .rhm)` triple.  Each entry's `status` is filled in
/// by opening the zip and either listing `.rhm` files (best case),
/// peeking each `.rhm`'s header to recover its proto-level filename
/// (also best case), or recording a reason for unsuitability.
///
/// Broken entries are still included so the picker can grey them out
/// with an explanation — silently hiding mods makes "why doesn't my
/// mod show up" undebuggable.
pub fn enumerate_missions(mods: &[DiscoveredMod], files: &SbFileSystem) -> Vec<MissionEntry> {
    let mut out = Vec::new();
    for m in mods {
        let preview = m.preview_image_path();
        if !m.details.hackable_missions.is_empty() {
            for mission in &m.details.hackable_missions {
                let descriptor = robin_engine::level_data::hackable_level_descriptor_path(mission);
                let status = match files.try_exists(&descriptor) {
                    Ok(true) => MissionStatus::Ok {
                        map_filename: mission.clone(),
                    },
                    Ok(false) => MissionStatus::Broken {
                        reason: format!("{descriptor} not found in any overlay datadir"),
                    },
                    Err(error) => MissionStatus::Broken {
                        reason: format!("{descriptor} lookup failed with file error {error}"),
                    },
                };
                out.push(MissionEntry {
                    mod_slug: m.details.slug.clone(),
                    mod_title: m.details.title.clone(),
                    author: m.details.author.clone(),
                    source_url: m.details.page_url.clone(),
                    license: m.details.license.clone(),
                    description: m.details.description.clone(),
                    map: m.details.map.clone(),
                    requires_spellforge: m.details.requires_spellforge(),
                    version_label: mission.clone(),
                    version_zip: PathBuf::new(),
                    rhm_zip_entry: String::new(),
                    rhm_basename: mission.clone(),
                    hackable: true,
                    status,
                    preview_image: preview.clone(),
                });
            }
            continue;
        }
        if m.details.versions.is_empty() {
            out.push(broken_entry(
                m,
                None,
                "no versions in details.json",
                preview.clone(),
            ));
            continue;
        }
        for version in &m.details.versions {
            let zip_path = m.version_zip_path(version);
            if !zip_path.is_file() {
                out.push(broken_entry(
                    m,
                    Some(version),
                    &format!("zip file missing: {}", zip_path.display()),
                    preview.clone(),
                ));
                continue;
            }
            let (mut archive, directory) =
                match open_mission_zip(&zip_path).and_then(|mut archive| {
                    let names = archive_directory(&mut archive)?;
                    Ok((archive, names))
                }) {
                    Ok(inspected) => inspected,
                    Err(e) => {
                        out.push(broken_entry(
                            m,
                            Some(version),
                            &format!("bad zip: {e}"),
                            preview.clone(),
                        ));
                        continue;
                    }
                };
            let rhm_entries = selectable_rhm_entries(&directory.names);
            if rhm_entries.is_empty() {
                out.push(broken_entry(
                    m,
                    Some(version),
                    "no .rhm files inside zip",
                    preview.clone(),
                ));
                continue;
            }
            let language_roots = rhm_entries
                .iter()
                .filter_map(|entry| mission_language_label(entry))
                .collect::<BTreeSet<_>>();
            let show_language = language_roots.len() > 1;
            for rhm_zip_entry in rhm_entries {
                let basename = rhm_basename(&rhm_zip_entry);
                let expected_mounted_rhm =
                    format!("data/levels/{}.rhm", basename.to_ascii_lowercase());
                let status = match selected_mission_layout(&directory.names, &rhm_zip_entry)
                    .and_then(|layout| {
                        if layout.mounted_rhm_path != expected_mounted_rhm {
                            return Err(format!(
                                "selected mission mounts as `{}`; gameplay requires `{expected_mounted_rhm}`",
                                layout.mounted_rhm_path
                            ));
                        }
                        peek_rhm_header(&mut archive, &directory, &rhm_zip_entry)
                    })
                {
                    Ok(header) => MissionStatus::Ok {
                        map_filename: header.map_filename,
                    },
                    Err(e) => MissionStatus::Broken {
                        reason: format!("mission validation failed: {e}"),
                    },
                };
                out.push(MissionEntry {
                    mod_slug: m.details.slug.clone(),
                    mod_title: m.details.title.clone(),
                    author: m.details.author.clone(),
                    source_url: m.details.page_url.clone(),
                    license: m.details.license.clone(),
                    description: m.details.description.clone(),
                    map: m.details.map.clone(),
                    requires_spellforge: m.details.requires_spellforge(),
                    version_label: if show_language {
                        mission_language_label(&rhm_zip_entry).map_or_else(
                            || version_label(version),
                            |language| format!("{} — {language}", version_label(version)),
                        )
                    } else {
                        version_label(version)
                    },
                    version_zip: zip_path.clone(),
                    rhm_zip_entry,
                    rhm_basename: basename,
                    hackable: false,
                    status,
                    preview_image: preview.clone(),
                });
            }
        }
    }
    out
}

fn broken_entry(
    m: &DiscoveredMod,
    version: Option<&ModVersion>,
    reason: &str,
    preview: Option<PathBuf>,
) -> MissionEntry {
    MissionEntry {
        mod_slug: m.details.slug.clone(),
        mod_title: m.details.title.clone(),
        author: m.details.author.clone(),
        source_url: m.details.page_url.clone(),
        license: m.details.license.clone(),
        description: m.details.description.clone(),
        map: m.details.map.clone(),
        requires_spellforge: m.details.requires_spellforge(),
        version_label: version.map(version_label).unwrap_or_else(|| "?".into()),
        version_zip: version.map(|v| m.version_zip_path(v)).unwrap_or_default(),
        rhm_zip_entry: String::new(),
        rhm_basename: String::new(),
        hackable: false,
        status: MissionStatus::Broken {
            reason: reason.to_string(),
        },
        preview_image: preview,
    }
}

fn version_label(v: &ModVersion) -> String {
    if v.version_notes.trim().is_empty() {
        v.date_uploaded.clone()
    } else {
        v.version_notes.clone()
    }
}

fn rhm_basename(zip_entry: &str) -> String {
    let leaf = zip_entry
        .rsplit_once('/')
        .map(|(_, leaf)| leaf)
        .unwrap_or(zip_entry);
    strip_rhm_extension(leaf).unwrap_or(leaf).to_owned()
}

/// Strip exactly one mission extension, using the same ASCII case policy as discovery.
fn strip_rhm_extension(name: &str) -> Option<&str> {
    let (stem, extension) = name.rsplit_once('.')?;
    extension.eq_ignore_ascii_case("rhm").then_some(stem)
}

fn mission_language_label(zip_entry: &str) -> Option<&str> {
    let mut segments = zip_entry.split('/');
    let mut previous = None;
    let mut current = segments.next()?;
    for next in segments {
        if current.eq_ignore_ascii_case("data") && next.eq_ignore_ascii_case("levels") {
            return previous;
        }
        previous = Some(current);
        current = next;
    }
    None
}

// ── Zip inspection ─────────────────────────────────────────────

/// List every selectable `.rhm` entry inside a zip.
///
/// Each returned path is later passed to the selected-mission overlay
/// mount, so multilingual archives expose every language instead of letting
/// central-directory order silently choose one.
pub fn list_rhm_in_zip(zip_path: &Path) -> Result<Vec<String>, String> {
    let mut archive = open_mission_zip(zip_path)?;
    let directory = archive_directory(&mut archive)?;
    Ok(selectable_rhm_entries(&directory.names))
}

fn selectable_rhm_entries(entry_names: &[String]) -> Vec<String> {
    let mut out = entry_names
        .iter()
        .filter(|name| strip_rhm_extension(name).is_some())
        .cloned()
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn open_mission_zip(zip_path: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("open: {e}"))?;
    zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))
}

/// One inspected central directory: display paths retain their spelling,
/// while lookup follows the overlay's normalized first-entry-wins policy.
#[derive(Debug, Serialize, Deserialize)]
struct ArchiveDirectory {
    names: Vec<String>,
    indices: HashMap<String, usize>,
}

fn archive_directory(archive: &mut zip::ZipArchive<fs::File>) -> Result<ArchiveDirectory, String> {
    let mut directory = ArchiveDirectory {
        names: Vec::with_capacity(archive.len()),
        indices: HashMap::with_capacity(archive.len()),
    };
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| format!("entry {i}: {e}"))?;
        if !entry.is_dir() && !entry.name().is_empty() {
            let name = entry.name().replace('\\', "/");
            directory
                .indices
                .entry(name.to_ascii_lowercase())
                .or_insert(i);
            directory.names.push(name);
        }
    }
    Ok(directory)
}

/// Exact selected archive layout reported by author tooling and used by the
/// mission mount path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedMissionLayout {
    pub strip_prefix: String,
    pub prepend_prefix: String,
    pub mounted_rhm_path: String,
}

pub fn selected_mission_layout_in_zip(
    zip_path: &Path,
    rhm_entry: &str,
) -> Result<SelectedMissionLayout, String> {
    let mut archive = open_mission_zip(zip_path)?;
    let directory = archive_directory(&mut archive)?;
    selected_mission_layout(&directory.names, rhm_entry)
}

fn selected_mission_layout(
    entries: &[String],
    rhm_entry: &str,
) -> Result<SelectedMissionLayout, String> {
    let (strip_prefix, prepend_prefix) = detect_zip_layout_for_mission(entries, rhm_entry)?;
    let selected = rhm_entry.to_ascii_lowercase();
    let relative = selected.strip_prefix(&strip_prefix).ok_or_else(|| {
        format!("selected mission `{rhm_entry}` is outside detected root `{strip_prefix}`")
    })?;
    Ok(SelectedMissionLayout {
        mounted_rhm_path: format!("{prepend_prefix}{relative}"),
        strip_prefix,
        prepend_prefix,
    })
}

/// Subset of `MissionHeader` we care about up-front: the proto-level
/// filename, which the engine pairs with the `.rhm` to build the map.
#[derive(Debug, Clone)]
pub struct RhmHeader {
    pub map_filename: String,
}

/// Read just enough of a `.rhm` to recover the proto-level filename.
///
/// The .rhm format begins with an outer chunk wrapper (file tag +
/// size + version, 12 bytes), then a HEAD/FOOT chunk wrapper (also
/// 12 bytes), then the header payload starting with `control_crc` and
/// `ambiance` (u32 each), then a length-prefixed `map_filename`
/// string (u16 LE length, then bytes).  We don't care about anything
/// past the map filename, so we read only the bytes we need.
pub fn peek_rhm_header_in_zip(zip_path: &Path, rhm_entry: &str) -> Result<RhmHeader, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    let directory = archive_directory(&mut archive)?;
    peek_rhm_header(&mut archive, &directory, rhm_entry)
}

// Outer wrapper, header wrapper, control CRC, ambiance, then a u16 byte length.
const RHM_NAME_LENGTH_OFFSET: usize = 12 + 12 + 4 + 4;
const RHM_HEADER_PREFIX_LEN: usize = RHM_NAME_LENGTH_OFFSET + 2;

fn peek_rhm_header(
    archive: &mut zip::ZipArchive<fs::File>,
    directory: &ArchiveDirectory,
    rhm_entry: &str,
) -> Result<RhmHeader, String> {
    // Match the overlay index: slash-normalized ASCII-insensitive paths,
    // with the first central-directory entry winning any aliases. Exact
    // by_name lookup can otherwise inspect different bytes than gameplay.
    let normalized = rhm_entry.replace('\\', "/").to_ascii_lowercase();
    let index = directory
        .indices
        .get(&normalized)
        .copied()
        .ok_or_else(|| format!("entry {rhm_entry}: not found in archive"))?;
    let mut entry = archive
        .by_index(index)
        .map_err(|e| format!("entry {rhm_entry}: {e}"))?;
    let mut buf = Vec::with_capacity(RHM_HEADER_PREFIX_LEN);
    entry
        .by_ref()
        .take(RHM_HEADER_PREFIX_LEN as u64)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read {rhm_entry}: {e}"))?;
    if buf.len() == RHM_HEADER_PREFIX_LEN {
        // A u16 bounds this read to 65,535 bytes even for an untrusted archive.
        // Leave the remaining mission payload unread.
        let name_len = rhm_name_length(&buf);
        entry
            .by_ref()
            .take(name_len as u64)
            .read_to_end(&mut buf)
            .map_err(|e| format!("read {rhm_entry}: {e}"))?;
    }
    parse_rhm_header(&buf)
}

fn rhm_name_length(prefix: &[u8]) -> usize {
    u16::from_le_bytes([
        prefix[RHM_NAME_LENGTH_OFFSET],
        prefix[RHM_NAME_LENGTH_OFFSET + 1],
    ]) as usize
}

fn parse_rhm_header(bytes: &[u8]) -> Result<RhmHeader, String> {
    if bytes.len() < RHM_HEADER_PREFIX_LEN {
        return Err(format!("file too short ({} bytes)", bytes.len()));
    }
    // Outer file tag must be a known mission marker.
    let outer_tag = &bytes[0..4];
    match outer_tag {
        b"RHMI" | b"DUTY" => {}
        other => {
            return Err(format!(
                "unknown mission tag {:?}",
                String::from_utf8_lossy(other)
            ));
        }
    }
    // Skip outer size+version (8 bytes). Inner chunk tag must be header.
    let inner_tag = &bytes[12..16];
    match inner_tag {
        b"HEAD" | b"FOOT" => {}
        other => {
            return Err(format!(
                "unexpected first inner chunk {:?}",
                String::from_utf8_lossy(other)
            ));
        }
    }
    // After inner tag+size+version (12 bytes) and crc+ambiance (8 bytes),
    // the next field is a u16 LE string length, then the bytes.
    let len = rhm_name_length(bytes);
    let str_start = RHM_HEADER_PREFIX_LEN;
    let str_end = str_start
        .checked_add(len)
        .ok_or_else(|| "string length overflow".to_string())?;
    if str_end > bytes.len() {
        return Err(format!(
            "map_filename truncated: len={len}, have={}",
            bytes.len() - str_start
        ));
    }
    let map_filename = String::from_utf8_lossy(&bytes[str_start..str_end]).into_owned();
    Ok(RhmHeader { map_filename })
}

// ── Mounting ────────────────────────────────────────────────────

/// RAII guard that holds zip overlays mounted via [`mount_for_launch`].
///
/// Dropping the guard removes each overlay in reverse mount order, so
/// the picker can return cleanly to an un-modded state when the player
/// quits a custom mission. The guard retains the exact preparation reader
/// it mounted into; neither mounting nor cleanup consults a process global.
#[must_use = "drop the guard to unmount overlays — leaks otherwise"]
pub struct MountGuard {
    overlays: Vec<String>,
    files: std::sync::Arc<SbFileSystem>,
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        for path in self.overlays.drain(..).rev() {
            let rc = self.files.remove_overlay(&path);
            if rc != SBFILE_NO_ERROR {
                tracing::warn!("MountGuard: remove_overlay({path}) returned {rc}");
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MountError {
    #[error("zip not found: {0}")]
    MissingZip(PathBuf),
    #[error("Spellforge lib zip not found under {0}")]
    MissingLib(PathBuf),
    #[error("SbFile::add_overlay_zip({0}) returned error code {1}")]
    OverlayAdd(PathBuf, i32),
    #[error("SbFile::add_overlay_zip_bytes({0}) returned error code {1}")]
    MemoryOverlayAdd(String, i32),
}

/// Mount exact validated archives directly from memory.
///
/// The immutable full-mod envelope owns these bytes for the whole mission,
/// while `SbFile` also retains an `Arc` inside each overlay. No browser-local
/// installation or filesystem API participates in lookup.
pub fn mount_distributed_archives(
    mount_namespace: &str,
    mission_archive: std::sync::Arc<[u8]>,
    shared_library_archive: Option<std::sync::Arc<[u8]>>,
    rhm_entry: &str,
    files: std::sync::Arc<SbFileSystem>,
) -> Result<MountGuard, MountError> {
    let mut guard = MountGuard {
        overlays: Vec::new(),
        files,
    };
    if let Some(shared) = shared_library_archive {
        let id = format!("memory://{mount_namespace}/spellforge-lib.zip");
        let rc = guard
            .files
            .add_overlay_zip_bytes_for_mission(&id, shared, None);
        if rc != SBFILE_NO_ERROR {
            return Err(MountError::MemoryOverlayAdd(id, rc));
        }
        guard.overlays.push(id);
    }
    let id = format!("memory://{mount_namespace}/mission.zip");
    let rc = guard
        .files
        .add_overlay_zip_bytes_for_mission(&id, mission_archive, Some(rhm_entry));
    if rc != SBFILE_NO_ERROR {
        return Err(MountError::MemoryOverlayAdd(id, rc));
    }
    guard.overlays.push(id);
    Ok(guard)
}

/// Mount a mod's version zip (and, for Spellforge mods, the shared
/// `lib/` zip) onto the SbFile overlay stack.  Mounting order matters:
/// the mod zip is pushed *last* so it overrides anything in lib.
pub fn mount_for_launch(
    version_zip: &Path,
    requires_spellforge: bool,
    mods_root: &Path,
    files: std::sync::Arc<SbFileSystem>,
) -> Result<MountGuard, MountError> {
    mount_for_launch_inner(version_zip, requires_spellforge, mods_root, None, files)
}

/// Mount a custom mission using its exact archive entry to select the
/// language or folder-wrapped datadir root.
pub fn mount_for_selected_mission(
    version_zip: &Path,
    requires_spellforge: bool,
    mods_root: &Path,
    rhm_entry: &str,
    files: std::sync::Arc<SbFileSystem>,
) -> Result<MountGuard, MountError> {
    mount_for_launch_inner(
        version_zip,
        requires_spellforge,
        mods_root,
        Some(rhm_entry),
        files,
    )
}

fn mount_for_launch_inner(
    version_zip: &Path,
    requires_spellforge: bool,
    mods_root: &Path,
    rhm_entry: Option<&str>,
    files: std::sync::Arc<SbFileSystem>,
) -> Result<MountGuard, MountError> {
    let mut guard = MountGuard {
        overlays: Vec::new(),
        files,
    };

    if requires_spellforge {
        let lib_zip = find_lib_zip(&mods_root.join("lib"))
            .ok_or_else(|| MountError::MissingLib(mods_root.join("lib")))?;
        let p = lib_zip.to_string_lossy().into_owned();
        let rc = guard.files.add_overlay_zip(&p);
        if rc != SBFILE_NO_ERROR {
            return Err(MountError::OverlayAdd(lib_zip, rc));
        }
        guard.overlays.push(p);
    }

    if !version_zip.is_file() {
        return Err(MountError::MissingZip(version_zip.to_path_buf()));
    }
    let p = version_zip.to_string_lossy().into_owned();
    let rc = match rhm_entry {
        Some(entry) => guard.files.add_overlay_zip_for_mission(&p, entry),
        None => guard.files.add_overlay_zip(&p),
    };
    if rc != SBFILE_NO_ERROR {
        return Err(MountError::OverlayAdd(version_zip.to_path_buf(), rc));
    }
    guard.overlays.push(p);

    Ok(guard)
}

/// Select the lexicographically greatest ZIP filename under `lib_dir`.
/// Upstream library uploads are normally date-stamped `lib_*.zip`, but
/// locally renamed ZIPs are accepted too. Extension matching ignores ASCII
/// case; filename ordering does not. Missing or empty directories yield `None`.
pub(crate) fn find_lib_zip(lib_dir: &Path) -> Option<PathBuf> {
    discovery_paths(lib_dir)
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f.to_ascii_lowercase().ends_with(".zip"))
        })
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_treats_non_directory_roots_as_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("not-a-directory");
        fs::write(&file, b"not a directory").unwrap();
        assert!(discovery_paths(&file).next().is_none());
        assert!(scan_mods_dir(&file).is_empty());
        assert_eq!(find_lib_zip(&file), None);
    }

    #[test]
    fn language_labels_keep_first_data_levels_pair_and_borrow_its_predecessor() {
        for (path, expected) in [
            ("English/Data/Levels/Test.rhm", Some("English")),
            ("package/Deutsch/dAtA/lEvElS/Test.rhm", Some("Deutsch")),
            ("日本語/Data/Levels/Test.rhm", Some("日本語")),
            ("Data/Levels/English/Data/Levels/Test.rhm", None),
            (
                "outer/Data/Levels/inner/Data/Levels/Test.rhm",
                Some("outer"),
            ),
            ("/Data/Levels/Test.rhm", Some("")),
            ("English//Data/Levels/Test.rhm", Some("")),
            ("English/Data/Other/Levels/Test.rhm", None),
            ("", None),
            ("Levels", None),
            ("Data", None),
        ] {
            let result = mission_language_label(path);
            assert_eq!(result, expected, "{path}");
            if let Some(label) = result {
                let start = path.as_ptr() as usize;
                assert!((start..=start + path.len()).contains(&(label.as_ptr() as usize)));
            }
        }
    }

    #[test]
    fn discover_and_enumerate_json_missions_from_directory_and_root_or_wrapped_zip() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let details = serde_json::json!({"slug":"gallery", "title":"Gallery", "page_url":"", "author":"test", "map":"test", "uploaded":"", "hackable_missions":["Gallery"]});
        let bytes = serde_json::to_vec(&details).unwrap();
        let directory = root.path().join("directory");
        fs::create_dir_all(directory.join("Data/Levels")).unwrap();
        fs::write(directory.join("details.json"), &bytes).unwrap();
        fs::write(directory.join("Data/Levels/Gallery.level.json"), b"{}").unwrap();
        for (name, prefix) in [("flat.zip", ""), ("wrapped.zip", "Wrapper/")] {
            let mut writer = zip::ZipWriter::new(fs::File::create(root.path().join(name)).unwrap());
            for (path, contents) in [
                ("details.json", bytes.as_slice()),
                ("Data/Levels/Gallery.level.json", b"{}".as_slice()),
            ] {
                writer
                    .start_file(
                        format!("{prefix}{path}"),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                writer.write_all(contents).unwrap();
            }
            writer.finish().unwrap();
        }
        let mods = scan_mods_dir(root.path());
        assert_eq!(mods.len(), 3);
        for discovered in mods {
            let files =
                SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
            assert_eq!(
                mount_mod_overlay(&files, &discovered.mod_dir),
                SBFILE_NO_ERROR
            );
            let entries = enumerate_missions(&[discovered], &files);
            assert_eq!(entries.len(), 1);
            assert!(entries[0].hackable);
            assert!(matches!(entries[0].status, MissionStatus::Ok { .. }));
        }
    }

    #[test]
    fn archive_directory_omits_directories_and_normalizes_file_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.zip");
        let file = fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.add_directory("Data/Levels/", options).unwrap();
        writer.start_file(r"Data\Levels\Test.rhm", options).unwrap();
        writer.start_file("notes.txt", options).unwrap();
        writer.finish().unwrap();
        let mut archive = open_mission_zip(&path).unwrap();
        let directory = archive_directory(&mut archive).unwrap();
        assert_eq!(directory.names, ["Data/Levels/Test.rhm", "notes.txt"]);
        // Omitting the directory from display names must not renumber ZIP entries.
        assert_eq!(directory.indices["data/levels/test.rhm"], 1);
        assert_eq!(directory.indices["notes.txt"], 2);
        assert!(!directory.indices.contains_key("data/levels/"));
        assert_eq!(list_rhm_in_zip(&path).unwrap(), ["Data/Levels/Test.rhm"]);
    }

    #[test]
    fn library_selection_uses_filename_order_and_accepts_renamed_archives() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path();
        assert_eq!(find_lib_zip(&directory.join("missing")), None);
        assert_eq!(find_lib_zip(directory), None);
        fs::create_dir(directory.join("zzzz.zip")).unwrap();
        fs::write(directory.join("zzzz.txt"), b"not an archive").unwrap();
        assert_eq!(find_lib_zip(directory), None);

        // Newer files on disk do not override lexicographically greater names.
        for name in ["lib_2026.zip", "lib_2025.ZIP", "LIB_9999.zip"] {
            fs::write(directory.join(name), b"archive candidate").unwrap();
        }
        assert_eq!(
            find_lib_zip(directory),
            Some(directory.join("lib_2026.zip"))
        );
        fs::write(directory.join("renamed.ZiP"), b"archive candidate").unwrap();
        assert_eq!(find_lib_zip(directory), Some(directory.join("renamed.ZiP")));
        fs::write(directory.join(".zip"), b"hidden archive candidate").unwrap();
        assert_eq!(find_lib_zip(directory), Some(directory.join("renamed.ZiP")));
    }

    #[test]
    fn combined_discovery_preserves_sources_and_skips_identical_roots() {
        let temporary = tempfile::tempdir().unwrap();
        let configured = temporary.path().join("configured");
        let bundled = temporary.path().join("bundled");
        for (root, slug, title) in [
            (&configured, "last", "Zulu"),
            (&configured, "same", "Shared"),
            (&bundled, "same", "Shared"),
            (&bundled, "first", "Alpha"),
        ] {
            let dir = root.join(slug);
            fs::create_dir_all(&dir).unwrap();
            let metadata = serde_json::json!({
                "slug": slug, "title": title, "page_url": "", "author": "",
                "map": "", "uploaded": ""
            });
            fs::write(
                dir.join("details.json"),
                serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        }
        let paths = |mods: Vec<DiscoveredMod>| {
            mods.into_iter()
                .map(|entry| entry.mod_dir)
                .collect::<Vec<_>>()
        };
        let configured_only = vec![configured.join("same"), configured.join("last")];
        assert_eq!(
            paths(scan_mission_roots(&configured, None)),
            configured_only
        );
        assert_eq!(
            paths(scan_mission_roots(&configured, Some(&configured))),
            configured_only
        );
        assert_eq!(
            paths(scan_mission_roots(&configured, Some(&bundled))),
            vec![
                bundled.join("first"),
                configured.join("same"),
                bundled.join("same"),
                configured.join("last"),
            ]
        );
        let missing = temporary.path().join("missing");
        assert_eq!(
            paths(scan_mission_roots(&configured, Some(&missing))),
            configured_only
        );
        assert_eq!(
            paths(scan_mission_roots(&missing, Some(&bundled))),
            vec![bundled.join("first"), bundled.join("same")]
        );
    }

    #[test]
    fn bundled_demos_use_the_supplied_overlay_filesystem() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods");
        let demos = scan_mods_dir(&root)
            .into_iter()
            .find(|entry| entry.details.slug == "multi-team-demos")
            .expect("bundled multi-team demos must be discoverable");
        let files = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let mods = [demos];
        let missing = enumerate_missions(&mods, &files);
        assert_eq!(missing.len(), 10);
        assert!(missing.iter().all(|entry| !entry.status.is_ok()));

        assert_eq!(
            files.add_overlay_path(mods[0].mod_dir.to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        let available = enumerate_missions(&mods, &files);
        assert_eq!(available.len(), 10);
        for entry in available {
            assert!(entry.hackable);
            assert!(
                entry.status.is_ok(),
                "{}: {:?}",
                entry.rhm_basename,
                entry.status
            );
        }
    }

    #[test]
    fn mount_guards_remove_only_their_own_preparation_overlays() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mission.zip");
        let rhm = minimal_rhm("lincoln");
        write_test_zip(&path, &[("Data/Levels/Test.rhm", &rhm)]);
        let bytes: std::sync::Arc<[u8]> = fs::read(path).unwrap().into();
        let make_files = || {
            std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
                robin_util::asset_fs::AssetVfs::new(),
            )))
        };
        let first = make_files();
        let second = make_files();
        let one = mount_distributed_archives(
            "same-name",
            bytes.clone(),
            None,
            "Data/Levels/Test.rhm",
            first.clone(),
        )
        .unwrap();
        let two = mount_distributed_archives(
            "same-name",
            bytes,
            None,
            "Data/Levels/Test.rhm",
            second.clone(),
        )
        .unwrap();
        assert_eq!(first.read_all("Data/Levels/Test.rhm").unwrap(), rhm);
        assert_eq!(second.read_all("Data/Levels/Test.rhm").unwrap(), rhm);
        let prepared = first.snapshot();
        drop(one);
        assert!(first.read_all("Data/Levels/Test.rhm").is_err());
        assert_eq!(prepared.read_all("Data/Levels/Test.rhm").unwrap(), rhm);
        assert_eq!(second.read_all("Data/Levels/Test.rhm").unwrap(), rhm);
        drop(two);
        assert!(second.read_all("Data/Levels/Test.rhm").is_err());
    }

    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;

        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn minimal_rhm(map: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DUTY");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(b"FOOT");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&(map.len() as u16).to_le_bytes());
        bytes.extend_from_slice(map.as_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes
    }

    #[test]
    fn discovery_checks_all_missions_with_shared_archive_despite_bad_header() {
        let root = tempfile::tempdir().unwrap();
        let zip_path = root.path().join("missions.zip");
        write_test_zip(
            &zip_path,
            &[
                ("Data/Levels/C.RhM", &minimal_rhm("third")),
                ("Data/Levels/A.rhm", b"broken"),
                ("Data/Levels/B.rhm.rhm", &minimal_rhm("second")),
            ],
        );
        let details: ModDetails = serde_json::from_value(serde_json::json!({
            "slug": "test", "title": "Test", "page_url": "", "author": "",
            "map": "", "uploaded": "",
            "versions": [{"date_uploaded": "1", "download_url": "", "local_file": "missions.zip"}]
        }))
        .unwrap();
        let files = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let rows = enumerate_missions(
            &[DiscoveredMod {
                details,
                mod_dir: root.path().to_path_buf(),
            }],
            &files,
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.rhm_zip_entry.clone())
                .collect::<Vec<_>>(),
            list_rhm_in_zip(&zip_path).unwrap()
        );
        assert_eq!(rows.len(), 3);
        for row in rows {
            let layout = selected_mission_layout_in_zip(&zip_path, &row.rhm_zip_entry).unwrap();
            assert_eq!(
                layout.mounted_rhm_path,
                format!("data/levels/{}.rhm", row.rhm_basename.to_ascii_lowercase())
            );
            match (
                row.status,
                peek_rhm_header_in_zip(&zip_path, &row.rhm_zip_entry),
            ) {
                (MissionStatus::Ok { map_filename }, Ok(header)) => {
                    assert_eq!(map_filename, header.map_filename)
                }
                (MissionStatus::Broken { reason }, Err(error)) => {
                    assert_eq!(reason, format!("mission validation failed: {error}"))
                }
                unexpected => {
                    panic!("discovery and standalone inspection disagree: {unexpected:?}")
                }
            }
        }
    }

    #[test]
    fn parse_vanilla() {
        let json = r#"{
          "slug": "derby-attack-siege",
          "title": "Derby Attack Siege",
          "page_url": "https://rhmods.com/missions/derby-attack-siege/",
          "author": "Nescafe",
          "map": "Derby",
          "uploaded": "Jan 7, 2025",
          "tags": ["Vanilla"],
          "likes": 1,
          "description": "I've created my own version of Derby siege",
          "images": ["https://example.com/img.png"],
          "versions": [{
            "date_uploaded": "Jan 7, 2025",
            "version_notes": "",
            "download_url": "https://example.com/x.zip",
            "local_file": "2025-01-07.zip"
          }]
        }"#;
        let d: ModDetails = serde_json::from_str(json).unwrap();
        assert_eq!(d.slug, "derby-attack-siege");
        assert!(!d.requires_spellforge());
        assert_eq!(d.versions[0].local_file, "2025-01-07.zip");
        assert!(d.hackable_missions.is_empty());
    }

    #[test]
    fn parse_multiple_hackable_missions() {
        let json = r#"{
          "slug": "gallery",
          "title": "Gallery",
          "page_url": "",
          "author": "Artist",
          "map": "OpenBattlefield",
          "uploaded": "Aug 27, 2026",
          "versions": [],
          "hackable_missions": ["GalleryAll", "GalleryDetail"]
        }"#;
        let details: ModDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.hackable_missions, ["GalleryAll", "GalleryDetail"]);
    }

    #[test]
    fn parse_spellforge() {
        let json = r#"{
          "slug": "meet-the-spy",
          "title": "Meet the Spy",
          "page_url": "https://rhmods.com/missions/meet-the-spy/",
          "author": "CraignRush",
          "map": "York",
          "uploaded": "Feb 12, 2026",
          "tags": ["Spellforge"],
          "likes": 2,
          "description": "...",
          "images": [],
          "versions": []
        }"#;
        let d: ModDetails = serde_json::from_str(json).unwrap();
        assert!(d.requires_spellforge());
    }

    #[test]
    fn rhm_basename_strips_path_and_extension() {
        assert_eq!(rhm_basename("S02_Lei_MP.rhm"), "S02_Lei_MP");
        assert_eq!(
            rhm_basename("English/DATA/Levels/CR02_Yrk_VL.rhm"),
            "CR02_Yrk_VL"
        );
        assert_eq!(rhm_basename("foo.RHM"), "foo");
        assert_eq!(rhm_basename("English/Data/Levels/Été.RhM"), "Été");
        assert_eq!(rhm_basename("foo.rhm.rhm"), "foo.rhm");
        assert_eq!(rhm_basename("foo.RHM.rhm"), "foo.RHM");
        assert_eq!(rhm_basename("foo"), "foo");
    }

    #[test]
    fn mission_extension_discovery_and_basename_agree() {
        let names: Vec<String> = [
            "Data/Levels/Été.RhM",
            "Data/Levels/foo.rhm.rhm",
            "Data/Levels/upper.RHM",
            "Data/Levels/lower.rhm",
            "Data/Levels/not.rhm.bak",
            "Data/Levels/no_extension",
            "Data/Levels/rhm",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let selected = selectable_rhm_entries(&names);
        let mut expected = names[..4].to_vec();
        expected.sort();
        assert_eq!(selected, expected);
        for name in selected {
            let leaf = name.rsplit('/').next().unwrap();
            let basename = rhm_basename(&name);
            assert!(format!("{basename}.rhm").eq_ignore_ascii_case(leaf));
        }
    }

    #[test]
    fn header_inspection_matches_mounted_archive_aliases() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mission.zip");
        let first = minimal_rhm("first");
        let second = minimal_rhm("second");
        write_test_zip(
            &path,
            &[
                (r"English\DATA\Levels\Test.RHM", &first),
                ("English/Data/Levels/Test.rhm", &second),
            ],
        );
        let files = std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        assert_eq!(
            peek_rhm_header_in_zip(&path, r"English\DATA\Levels\Test.RHM")
                .unwrap()
                .map_filename,
            "first",
        );
        let entries = list_rhm_in_zip(&path).unwrap();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            let header = peek_rhm_header_in_zip(&path, &entry).unwrap();
            let layout = selected_mission_layout_in_zip(&path, &entry).unwrap();
            let _guard =
                mount_for_selected_mission(&path, false, root.path(), &entry, files.clone())
                    .unwrap();
            let mounted = files.read_all(&layout.mounted_rhm_path).unwrap();
            assert_eq!(mounted, first);
            assert_eq!(
                header.map_filename,
                parse_rhm_header(&mounted).unwrap().map_filename
            );
        }
        assert!(
            peek_rhm_header_in_zip(&path, "missing.rhm")
                .unwrap_err()
                .contains("not found in archive")
        );
    }

    #[test]
    fn zip_header_reader_honors_declared_name_length() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mission.zip");
        for len in [0, 10, 222, 223, usize::from(u16::MAX)] {
            let name = "x".repeat(len);
            write_test_zip(&path, &[("Data/Levels/Test.rhm", &minimal_rhm(&name))]);
            let header = peek_rhm_header_in_zip(&path, "Data/Levels/Test.rhm").unwrap();
            assert_eq!(header.map_filename, name);
        }
    }

    #[test]
    fn zip_header_reader_keeps_parser_truncation_errors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mission.zip");
        let bytes = minimal_rhm("longer");
        for len in [
            0,
            12,
            RHM_HEADER_PREFIX_LEN - 1,
            RHM_HEADER_PREFIX_LEN,
            RHM_HEADER_PREFIX_LEN + 2,
        ] {
            let truncated = &bytes[..len];
            write_test_zip(&path, &[("Data/Levels/Test.rhm", truncated)]);
            assert_eq!(
                peek_rhm_header_in_zip(&path, "Data/Levels/Test.rhm").unwrap_err(),
                parse_rhm_header(truncated).unwrap_err(),
            );
        }
    }

    #[test]
    fn parse_rhm_header_minimal() {
        // Construct a synthetic .rhm header: outer DUTY chunk + FOOT
        // header chunk with map_filename = "MyMap".
        let bytes = minimal_rhm("MyMap");
        let h = parse_rhm_header(&bytes).unwrap();
        assert_eq!(h.map_filename, "MyMap");
    }

    #[test]
    fn parse_rhm_header_rejects_unknown_tag() {
        let mut bytes = vec![b'X'; 40];
        let err = parse_rhm_header(&bytes).unwrap_err();
        assert!(err.contains("unknown mission tag"), "got: {err}");
        // Truncated input.
        bytes.truncate(10);
        let err = parse_rhm_header(&bytes).unwrap_err();
        assert!(err.contains("too short"), "got: {err}");
    }

    #[test]
    fn multilingual_archive_exposes_every_exact_mission_root() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("rescue.zip");
        write_test_zip(
            &archive,
            &[
                ("English/DATA/Levels/H06_Lin_VL.rhm", b"english"),
                ("German/DATA/Levels/H06_Lin_VL.rhm", b"german"),
                ("Polish/DATA/Levels/H06_Lin_VL.rhm", b"polish"),
            ],
        );

        assert_eq!(
            list_rhm_in_zip(&archive).unwrap(),
            [
                "English/DATA/Levels/H06_Lin_VL.rhm",
                "German/DATA/Levels/H06_Lin_VL.rhm",
                "Polish/DATA/Levels/H06_Lin_VL.rhm",
            ]
        );
        let german =
            selected_mission_layout_in_zip(&archive, "German/DATA/Levels/H06_Lin_VL.rhm").unwrap();
        assert_eq!(german.strip_prefix, "german/");
        assert_eq!(german.prepend_prefix, "");
        assert_eq!(german.mounted_rhm_path, "data/levels/h06_lin_vl.rhm");
    }

    #[test]
    fn multilingual_archive_creates_one_launchable_picker_row_per_language() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("rescue.zip");
        let rhm = minimal_rhm("H06_Lin_VL");
        write_test_zip(
            &archive,
            &[
                ("English/DATA/Levels/H06_Lin_VL.rhm", rhm.as_slice()),
                ("German/DATA/Levels/H06_Lin_VL.rhm", rhm.as_slice()),
                ("Polish/DATA/Levels/H06_Lin_VL.rhm", rhm.as_slice()),
            ],
        );
        let discovered = DiscoveredMod {
            details: ModDetails {
                slug: "rescue".to_owned(),
                title: "Rescue".to_owned(),
                page_url: String::new(),
                author: "Author".to_owned(),
                license: "CC0-1.0".to_owned(),
                map: "Lincoln".to_owned(),
                uploaded: "today".to_owned(),
                tags: vec!["Spellforge".to_owned()],
                likes: 0,
                description: String::new(),
                images: Vec::new(),
                versions: vec![ModVersion {
                    date_uploaded: "today".to_owned(),
                    version_notes: "v1".to_owned(),
                    download_url: String::new(),
                    local_file: "rescue.zip".to_owned(),
                }],
                hackable_missions: Vec::new(),
            },
            mod_dir: tmp.path().to_owned(),
        };

        let files = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        let rows = enumerate_missions(&[discovered], &files);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.status.is_ok()));
        assert_eq!(
            rows.iter()
                .map(|row| row.version_label.as_str())
                .collect::<Vec<_>>(),
            ["v1 — English", "v1 — German", "v1 — Polish"]
        );
    }

    #[test]
    fn folder_wrapped_mission_layout_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("first-lincoln.zip");
        write_test_zip(
            &archive,
            &[
                ("H01_Lin_VL/H01_Lin_VL.rhm", b"rhm"),
                ("H01_Lin_VL/H01_Lin_VL.lua", b"lua"),
                ("H01_Lin_VL/enums.lua", b"enums"),
            ],
        );

        let layout = selected_mission_layout_in_zip(&archive, "H01_Lin_VL/H01_Lin_VL.rhm").unwrap();
        assert_eq!(layout.strip_prefix, "h01_lin_vl/");
        assert_eq!(layout.prepend_prefix, "data/levels/");
        assert_eq!(layout.mounted_rhm_path, "data/levels/h01_lin_vl.rhm");
    }
}
