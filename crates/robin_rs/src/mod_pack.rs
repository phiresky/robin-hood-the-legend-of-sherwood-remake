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
//!   `SbFile::add_overlay_zip`, with the Spellforge `lib/` zip layered
//!   underneath when needed
//!
//! Lua / Spellforge runtime support is the job of a separate agent —
//! this module is purely concerned with discovery and file-system layering.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use robin_engine::sbfile::{SBFILE_NO_ERROR, SbFile, detect_zip_layout};

/// Top-level metadata for one custom mission mod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetails {
    pub slug: String,
    pub title: String,
    pub page_url: String,
    pub author: String,
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
    /// When set, this mod is a hackable JSON level shipped as an
    /// always-mounted overlay datadir (see [`crate::main_entry`]'s
    /// `MODS_DIR`) rather than a downloadable zip: the value is the
    /// `<mission>` of a `Data/Levels/<mission>.level.json` descriptor.
    /// Such mods need no `versions` — the picker launches them through
    /// the hackable-level path instead of mounting a zip.
    #[serde(default)]
    pub hackable_mission: Option<String>,
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

/// A mod discovered on disk: its parsed `details.json` plus the
/// directory it lives in (so the per-mod assets — zip files, locally
/// cached preview images — can be located relative to it).
#[derive(Debug, Clone)]
pub struct DiscoveredMod {
    pub details: ModDetails,
    pub mod_dir: PathBuf,
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

/// Scan `mods_root` for `<slug>/details.json` files, returning all
/// successfully-parsed mods.  Parse failures are logged at `warn` and
/// skipped — a bad single `details.json` shouldn't make the entire
/// picker unavailable.
pub fn scan_mods_dir(mods_root: &Path) -> Vec<DiscoveredMod> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(mods_root) else {
        tracing::info!(
            "scan_mods_dir: {} not readable, no custom missions available",
            mods_root.display()
        );
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let details_path = path.join("details.json");
        if !details_path.is_file() {
            continue;
        }
        match ModDetails::load(&details_path) {
            Ok(details) => out.push(DiscoveredMod {
                details,
                mod_dir: path,
            }),
            Err(e) => {
                tracing::warn!("scan_mods_dir: skipping {}: {e}", details_path.display());
            }
        }
    }
    out.sort_by(|a, b| a.details.title.cmp(&b.details.title));
    out
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

    /// True for hackable JSON levels (`ModDetails::hackable_mission`):
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
pub fn enumerate_missions(mods: &[DiscoveredMod]) -> Vec<MissionEntry> {
    let mut out = Vec::new();
    for m in mods {
        let preview = m.preview_image_path();
        if let Some(mission) = &m.details.hackable_mission {
            // Hackable JSON level: no zip to enumerate. Launchable iff
            // the descriptor resolves through the mounted overlays.
            let status = if robin_engine::level_data::hackable_level_exists(mission) {
                MissionStatus::Ok {
                    map_filename: mission.clone(),
                }
            } else {
                MissionStatus::Broken {
                    reason: format!(
                        "Data/Levels/{mission}.level.json not found in any overlay datadir"
                    ),
                }
            };
            out.push(MissionEntry {
                mod_slug: m.details.slug.clone(),
                mod_title: m.details.title.clone(),
                author: m.details.author.clone(),
                description: m.details.description.clone(),
                map: m.details.map.clone(),
                requires_spellforge: m.details.requires_spellforge(),
                version_label: "overlay".to_string(),
                version_zip: PathBuf::new(),
                rhm_zip_entry: String::new(),
                rhm_basename: mission.clone(),
                hackable: true,
                status,
                preview_image: preview,
            });
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
            let rhm_entries = match list_rhm_in_zip(&zip_path) {
                Ok(v) => v,
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
            if rhm_entries.is_empty() {
                out.push(broken_entry(
                    m,
                    Some(version),
                    "no .rhm files inside zip",
                    preview.clone(),
                ));
                continue;
            }
            for rhm_zip_entry in rhm_entries {
                let basename = rhm_basename(&rhm_zip_entry);
                let status = match peek_rhm_header_in_zip(&zip_path, &rhm_zip_entry) {
                    Ok(header) => MissionStatus::Ok {
                        map_filename: header.map_filename,
                    },
                    Err(e) => MissionStatus::Broken {
                        reason: format!("rhm header parse failed: {e}"),
                    },
                };
                out.push(MissionEntry {
                    mod_slug: m.details.slug.clone(),
                    mod_title: m.details.title.clone(),
                    author: m.details.author.clone(),
                    description: m.details.description.clone(),
                    map: m.details.map.clone(),
                    requires_spellforge: m.details.requires_spellforge(),
                    version_label: version_label(version),
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
    leaf.trim_end_matches(".rhm")
        .trim_end_matches(".RHM")
        .to_string()
}

// ── Zip inspection ─────────────────────────────────────────────

/// List every `.rhm` entry inside a zip that the overlay would
/// actually expose at a datadir-style path.
///
/// Some zips bundle the same mission file in multiple language
/// folders (e.g. `English/`, `German/`, `Polish/` siblings).  The
/// engine's overlay picks one of those as the datadir root via
/// [`detect_zip_layout`]; the other variants would still appear in a
/// naive entry listing but would never be loadable.  We use the same
/// detection here so the picker only shows rows that map to a
/// reachable file.
pub fn list_rhm_in_zip(zip_path: &Path) -> Result<Vec<String>, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("open: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    let mut entry_names: Vec<String> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| format!("entry {i}: {e}"))?;
        if entry.is_dir() {
            entry_names.push(String::new());
        } else {
            entry_names.push(entry.name().replace('\\', "/"));
        }
    }
    let (strip, _prepend) = detect_zip_layout(&entry_names);
    let mut out = Vec::new();
    for name in &entry_names {
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".rhm") {
            continue;
        }
        if !strip.is_empty() && !lower.starts_with(&strip) {
            continue;
        }
        out.push(name.clone());
    }
    out.sort();
    Ok(out)
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
    let mut entry = archive
        .by_name(rhm_entry)
        .map_err(|e| format!("entry {rhm_entry}: {e}"))?;
    let mut buf = Vec::with_capacity(256);
    // 12 (outer) + 12 (header chunk) + 4 (crc) + 4 (ambiance) + 2 (str len) = 34
    // bytes minimum; string follows, plus 4 bytes profile_id which we skip.
    // Mission filenames are short (~10 chars), so reading 256 bytes covers
    // all real cases.
    entry
        .by_ref()
        .take(256)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read {rhm_entry}: {e}"))?;
    parse_rhm_header(&buf)
}

fn parse_rhm_header(bytes: &[u8]) -> Result<RhmHeader, String> {
    if bytes.len() < 34 {
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
    let str_off = 12 + 12 + 4 + 4;
    let len = u16::from_le_bytes([bytes[str_off], bytes[str_off + 1]]) as usize;
    let str_start = str_off + 2;
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
/// quits a custom mission.
#[must_use = "drop the guard to unmount overlays — leaks otherwise"]
pub struct MountGuard {
    overlays: Vec<String>,
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        for path in self.overlays.drain(..).rev() {
            let rc = SbFile::remove_overlay(&path);
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
}

/// Mount a mod's version zip (and, for Spellforge mods, the shared
/// `lib/` zip) onto the SbFile overlay stack.  Mounting order matters:
/// the mod zip is pushed *last* so it overrides anything in lib.
pub fn mount_for_launch(
    version_zip: &Path,
    requires_spellforge: bool,
    mods_root: &Path,
) -> Result<MountGuard, MountError> {
    let mut guard = MountGuard {
        overlays: Vec::new(),
    };

    if requires_spellforge {
        let lib_zip = find_lib_zip(&mods_root.join("lib"))
            .ok_or_else(|| MountError::MissingLib(mods_root.join("lib")))?;
        let p = lib_zip.to_string_lossy().into_owned();
        let rc = SbFile::add_overlay_zip(&p);
        if rc != SBFILE_NO_ERROR {
            return Err(MountError::OverlayAdd(lib_zip, rc));
        }
        guard.overlays.push(p);
    }

    if !version_zip.is_file() {
        return Err(MountError::MissingZip(version_zip.to_path_buf()));
    }
    let p = version_zip.to_string_lossy().into_owned();
    let rc = SbFile::add_overlay_zip(&p);
    if rc != SBFILE_NO_ERROR {
        return Err(MountError::OverlayAdd(version_zip.to_path_buf(), rc));
    }
    guard.overlays.push(p);

    Ok(guard)
}

/// Find the newest `lib_*.zip` under `lib_dir`, by name (the upstream
/// uploads are date-stamped).  Returns `None` if the directory is
/// missing or empty.
fn find_lib_zip(lib_dir: &Path) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(lib_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f.to_ascii_lowercase().ends_with(".zip"))
        })
        .collect();
    entries.sort();
    entries.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn parse_rhm_header_minimal() {
        // Construct a synthetic .rhm header: outer DUTY chunk + FOOT
        // header chunk with map_filename = "MyMap".
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DUTY"); // outer tag
        bytes.extend_from_slice(&0u32.to_le_bytes()); // outer size (ignored)
        bytes.extend_from_slice(&2u32.to_le_bytes()); // outer version
        bytes.extend_from_slice(b"FOOT"); // header tag
        bytes.extend_from_slice(&0u32.to_le_bytes()); // header size (ignored)
        bytes.extend_from_slice(&4u32.to_le_bytes()); // header version
        bytes.extend_from_slice(&0u32.to_le_bytes()); // control_crc
        bytes.extend_from_slice(&5u32.to_le_bytes()); // ambiance
        let name = b"MyMap";
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // profile_id

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
}
