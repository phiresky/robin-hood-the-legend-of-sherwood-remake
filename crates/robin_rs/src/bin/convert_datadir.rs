//! Datadir format converter.
//!
//! Converts a legacy (original) datadir into a new datadir format:
//!   - `hackable` — JSON + lossless PNGs, human-readable and editable
//!   - `shipping` — compact packed format (see `convert_shipping`), aimed at
//!     small download size; long-term target is bitcode + zstd(22, long=31).
//!
//! The converter does **not** walk the input tree. It starts from a small set
//! of hardcoded root paths and follows references discovered by the existing
//! parsers (profile.cpf → missions/characters, levels → sprites/maps/sounds).
//! Files never referenced by any index are considered unused and dropped.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use robin_rs::frame_holder::{FrameHolder, SpriteVariant};
use robin_rs::level_loader::{
    ChunkReader, LevelFormat, LoadedMission, LoadedProtoLevel, load_mission, load_proto_level,
};
use robin_rs::main_entry::{FALLBACK_LOCALE_FOLDER, LANGUAGE_FOLDERS};
use robin_rs::order::OrderType;
use robin_rs::picture::Picture;
use robin_rs::profiles::{CivilianType, ProfileManager};
use robin_rs::res_descr;
use robin_rs::resource_manager::{EncodedPicture, ResourceManager};
use robin_rs::sbfile::{SB_FILE_READ, SbFile, resolve_case_insensitive};
use robin_rs::scb;
use robin_rs::sprite_scriptor;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutFormat {
    /// JSON + PNGs, human-readable and hackable.
    Hackable,
    /// Compact packed format, shipping-optimized.
    Shipping,
}

#[derive(Parser, Debug)]
#[command(about = "Convert a legacy Robin Hood datadir to a new format.")]
struct Args {
    /// Path to the original datadir (the directory containing `DATA/` or `Data/`).
    #[arg(short, long)]
    input: PathBuf,

    /// Destination directory. Created fresh unless `--force`.
    #[arg(short, long)]
    output: PathBuf,

    /// Target format.
    #[arg(short, long, value_enum, default_value_t = OutFormat::Hackable)]
    format: OutFormat,

    /// Overwrite `output` if it exists.
    #[arg(long)]
    force: bool,

    /// Shipping: how to encode `.map` / `.min` terrain bitmaps.
    /// `raw` keeps the original bzip2-RGB565 bytes (current behavior);
    /// `jxl-lossless` transcodes them via `cjxl -d 0 --modular=1`; `jxl-q90`
    /// transcodes via `cjxl -q 90` (~60% smaller, visually lossless).
    /// `jxl-q85` / `jxl-q80` trade more terrain-map fidelity for smaller blobs.
    #[arg(long, value_enum, default_value_t = MapFormat::Raw)]
    map_format: MapFormat,

    /// Shipping: how to encode picture payloads inside interface `.res` /
    /// `.pak` bundles. `raw` keeps RGB565 bytes; `jxl-lossless` keeps exact
    /// RGBA values; `jxl-q80` is the current size-oriented target.
    #[arg(long, value_enum, default_value_t = InterfaceImageFormat::Raw)]
    interface_image_format: InterfaceImageFormat,

    /// Shipping: cap the zstd `windowLog` parameter. Defaults to 31; set to 30
    /// for wasm32 targets (32-bit zstd builds can't decode long=31 streams).
    #[arg(long, default_value_t = 31)]
    zstd_window_log: u32,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum MapFormat {
    /// Shipping stores the original bzip2-compressed RGB565 `.map` bytes.
    Raw,
    /// Shipping transcodes `.map` files to lossless JXL (modular).
    JxlLossless,
    /// Shipping transcodes `.map` files to JXL quality 90 (visually lossless).
    JxlQ90,
    /// Shipping transcodes `.map` files to JXL quality 85.
    JxlQ85,
    /// Shipping transcodes `.map` files to JXL quality 80.
    JxlQ80,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum InterfaceImageFormat {
    /// Keep interface resource pictures as raw RGB565.
    Raw,
    /// Encode interface resource pictures as lossless JXL.
    JxlLossless,
    /// Encode interface resource pictures as JXL quality 90.
    JxlQ90,
    /// Encode interface resource pictures as JXL quality 85.
    JxlQ85,
    /// Encode interface resource pictures as JXL quality 80.
    JxlQ80,
}

impl InterfaceImageFormat {
    fn jxl_quality(self) -> Option<Option<u8>> {
        match self {
            Self::Raw => None,
            Self::JxlLossless => Some(None),
            Self::JxlQ90 => Some(Some(90)),
            Self::JxlQ85 => Some(Some(85)),
            Self::JxlQ80 => Some(Some(80)),
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    if !args.input.is_dir() {
        bail!("input is not a directory: {}", args.input.display());
    }
    if args.output.exists() {
        if args.force {
            fs::remove_dir_all(&args.output)?;
        } else {
            bail!(
                "output exists (pass --force to overwrite): {}",
                args.output.display()
            );
        }
    }
    fs::create_dir_all(&args.output)?;

    let data_in = find_data_dir(&args.input)?;
    let data_out = args.output.join("Data");
    fs::create_dir_all(&data_out)?;

    match args.format {
        OutFormat::Hackable => Converter::new(data_in, data_out).run(),
        OutFormat::Shipping => convert_shipping(
            data_in,
            &data_out,
            ShippingOpts {
                map_format: args.map_format,
                interface_image_format: args.interface_image_format,
                zstd_window_log: args.zstd_window_log,
            },
        ),
    }
}

#[derive(Debug, Clone, Copy)]
struct ShippingOpts {
    map_format: MapFormat,
    interface_image_format: InterfaceImageFormat,
    zstd_window_log: u32,
}

/// Locate the game data directory inside the input folder.
/// The original datadir capitalization varies (`DATA/` in demos, `Data/` in fullgame).
fn find_data_dir(input: &Path) -> Result<PathBuf> {
    for name in ["Data", "DATA", "data"] {
        let p = input.join(name);
        if p.is_dir() {
            return Ok(p);
        }
    }
    bail!(
        "no Data/ directory found inside {} (expected Data/, DATA/, or data/)",
        input.display()
    )
}

/// Windows LCID → BCP-47 / ISO locale string.  Used to rename the
/// localized subfolders in the hackable output so they're readable
/// (`1033` → `en-US`).  Unknown LCIDs fall through to the numeric name.
fn lcid_to_iso(lcid: &str) -> &'static str {
    match lcid {
        "1028" => "zh-TW",
        "1029" => "cs-CZ",
        "1031" => "de-DE",
        "1033" => "en-US",
        "1036" => "fr-FR",
        "1040" => "it-IT",
        "1041" => "ja-JP",
        "1042" => "ko-KR",
        "1045" => "pl-PL",
        "1046" => "pt-BR",
        "1049" => "ru-RU",
        "1054" => "th-TH",
        "2047" => "neutral",
        "2052" => "zh-CN",
        "2070" => "pt-PT",
        "3082" => "es-ES",
        // Unknown — keep numeric so the conversion is never lossy.
        _ => Box::leak(lcid.to_string().into_boxed_str()),
    }
}

/// Resolve `<root>/<lcid>/Data` (case-insensitive on both components) to a
/// real directory if it exists.  Returns `None` otherwise.
fn resolve_locale_data_dir(root: &Path, lcid: &str) -> Option<PathBuf> {
    let lcid_dir = resolve_case_insensitive(&root.join(lcid))?;
    if !lcid_dir.is_dir() {
        return None;
    }
    for name in ["Data", "DATA", "data"] {
        let p = lcid_dir.join(name);
        if p.is_dir() {
            return Some(p);
        }
        if let Some(resolved) = resolve_case_insensitive(&p)
            && resolved.is_dir()
        {
            return Some(resolved);
        }
    }
    None
}

/// A locale alternate source dir + the ISO name used for its output subtree.
#[derive(Debug, Clone)]
struct LocaleSource {
    data_dir: PathBuf,
    iso: &'static str,
}

/// Detect locale data dirs alongside `data_in`, mirroring the runtime logic
/// in `main_entry::add_language_folder`: always probe the English fallback
/// (`1033`), then the first existing entry from `LANGUAGE_FOLDERS`.
fn detect_locale_data_dirs(data_in: &Path) -> Vec<LocaleSource> {
    let Some(root) = data_in.parent() else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    if let Some(d) = resolve_locale_data_dir(root, FALLBACK_LOCALE_FOLDER) {
        sources.push(LocaleSource {
            data_dir: d,
            iso: lcid_to_iso(FALLBACK_LOCALE_FOLDER),
        });
    }
    for &folder in LANGUAGE_FOLDERS {
        if let Some(d) = resolve_locale_data_dir(root, folder) {
            sources.push(LocaleSource {
                data_dir: d,
                iso: lcid_to_iso(folder),
            });
            break;
        }
    }
    sources
}

/// Result of `Converter::in_path`: the resolved source path plus, if it
/// was found under a locale alt-dir, the ISO name of that locale so the
/// converter can place the output in the matching `<iso>/Data/` subtree.
#[derive(Debug)]
struct Resolved {
    src: PathBuf,
    locale: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Converter state
// ---------------------------------------------------------------------------

struct Converter {
    data_in: PathBuf,
    data_out: PathBuf,
    /// Locale-specific data dirs probed after `data_in` when resolving a
    /// relative path.  Mirrors the runtime `SbFile` alternate-path mechanism
    /// set up by `main_entry::add_language_folder`: `<root>/1033/Data` plus
    /// whichever other `LANGUAGE_FOLDERS` entry ships with the datadir.
    /// Files that resolve via a locale source land in the output under
    /// `<output>/<iso>/Data/<rel>` so the per-locale structure is preserved.
    locale_data_dirs: Vec<LocaleSource>,
    /// Needed to drive `load_mission`'s `is_beggar` predicate.
    beggar_civ_indices: Arc<BTreeSet<u32>>,
    /// Lazy-loaded shared sprite bank (`robinhood.bks` + `robinhood.dic`).
    /// Frames from this bank are extracted into each `.rhs.d/` directory as
    /// they're referenced — the bank itself never appears in the output.
    frame_holder: Option<FrameHolder>,
    /// Bank sprite indices we've written at least once. Any sprite in the
    /// bank that's never referenced by a converted `.rhs` gets dumped into
    /// `_unused_sprites/` at the end so data is never silently dropped.
    used_sprites: BTreeSet<u32>,
    converted: usize,
    copied: usize,
    missing: usize,
}

impl Converter {
    fn new(data_in: PathBuf, data_out: PathBuf) -> Self {
        let locale_data_dirs = detect_locale_data_dirs(&data_in);
        for src in &locale_data_dirs {
            tracing::info!("Locale data dir [{}]: {}", src.iso, src.data_dir.display());
        }
        Self {
            data_in,
            data_out,
            locale_data_dirs,
            beggar_civ_indices: Arc::new(BTreeSet::new()),
            frame_holder: None,
            used_sprites: BTreeSet::new(),
            converted: 0,
            copied: 0,
            missing: 0,
        }
    }

    fn run(mut self) -> Result<()> {
        // ── Pass 1 : fixed boot roots ─────────────────────────────────
        // Paths are relative to the Data/ dir and come from hardcoded
        // strings in the engine (main_entry.rs, loading_screen, etc.).
        // NOTE: `robinhood.bks` + `robinhood.dic` are *not* roots. They're
        // a shared sprite pool that only makes sense in the context of the
        // `.rhs` files that reference bank IDs, so we explode those frames
        // into each `.rhs.d/` directory when converting.
        // Boot-time resource files attached at launch
        // (`Data/Text/Level.res`, `Data/Interface/DEFAULT.RES`,
        // `Data/Sounds/Exclamations/actors.res`) plus the expression/actor
        // text table (`Text/actors.res`) and the loading-screen bundle.
        // `Text/Level.res` is only shipped under the locale subfolder
        // (e.g. `1033/Data/Text/Level.res`), so it depends on the
        // alternate-path resolution in `in_path`.
        for p in [
            "Interface/DEFAULT.RES",
            "Interface/Loading.pak",
            "Text/actors.res",
            "Text/Level.res",
            "Sounds/Exclamations/actors.res",
        ] {
            self.convert_rel(p)?;
        }

        // ── Pass 2 : profile.cpf (root index) and its references ──────
        let cpf_rel = "Configuration/profile.cpf";
        let cpf = self.load_and_convert_cpf(cpf_rel)?;

        // Update the beggar predicate now that we know civilian types.
        self.beggar_civ_indices = Arc::new(
            cpf.civilians
                .iter()
                .enumerate()
                .filter_map(|(i, c)| (c.civilian_type == CivilianType::Beggar).then_some(i as u32))
                .collect(),
        );

        // Character-style entries all live in Data/Characters/<filename>.rhs.
        let mut chars: BTreeSet<String> = BTreeSet::new();
        for c in &cpf.characters {
            chars.insert(c.filename.clone());
        }
        for s in &cpf.soldiers {
            chars.insert(s.filename.clone());
        }
        for c in &cpf.civilians {
            chars.insert(c.filename.clone());
        }
        for name in &chars {
            if name.is_empty() {
                continue;
            }
            self.convert_rel(&format!("Characters/{name}.rhs"))?;
        }

        // Missions: proto-level (.rhp), mission (.rhm), script (.scb).
        let mut level_refs = LevelRefs::default();
        for mp in &cpf.missions {
            if mp.proto_level_filename.is_empty() || mp.mission_filename.is_empty() {
                continue;
            }
            self.convert_rel(&format!("Levels/{}.rhp", mp.proto_level_filename))?;
            self.convert_rel(&format!("Levels/{}.rhm", mp.mission_filename))?;
            self.convert_rel(&format!("Levels/{}.scb", mp.mission_filename))?;
            // Per-mission level descriptor (e.g. RHLevelSB.red). Filename
            // is derived from the mission id.
            let red_rel = format!("Text/{}", res_descr::red_filename(mp.id));
            self.convert_rel(&red_rel)?;

            match self.parse_level(&mp.proto_level_filename, &mp.mission_filename) {
                Ok((proto, mission)) => collect_level_refs(&proto, &mission, &mut level_refs),
                Err(e) => tracing::warn!(
                    "could not parse level {}/{}: {:#}",
                    mp.proto_level_filename,
                    mp.mission_filename,
                    e
                ),
            }
        }

        // ── Pass 3 : level references (sprites + terrain maps) ────────
        for sprite in &level_refs.sprite_rhs {
            self.convert_rel(&format!("Characters/{sprite}.rhs"))?;
        }
        for map in &level_refs.map_names {
            // The map/min files are stored under an ambience subdirectory.
            // The ambience isn't in the level-refs index, so we probe each
            // known subdir; any that resolves gets converted. Converting
            // extra ambiences is harmless.
            for sub in ["Day", "Night", "Fog"] {
                for ext in [".map", ".min"] {
                    let rel = format!("Levels/{sub}/{map}{ext}");
                    if self.exists(&rel) {
                        self.convert_rel(&rel)?;
                    }
                }
            }
        }
        // Sound-source waves — `snd_NNN.wav` under Data/Sounds/. Not
        // every referenced id ships with a file (some optional samples
        // are missing from the demo), so `convert_rel` logs a
        // warning-plus-`self.missing++` rather than hard-failing.
        for &id in &level_refs.sound_wave_ids {
            let rel = format!("Sounds/snd_{id:03}.wav");
            if self.exists(&rel) {
                self.convert_rel(&rel)?;
            }
        }

        // ── Final pass : dump sprites that no `.rhs` referenced ───────
        self.dump_unused_sprites()?;

        tracing::info!(
            "done: converted={} copied={} missing={}",
            self.converted,
            self.copied,
            self.missing
        );
        Ok(())
    }

    // ── File helpers ──────────────────────────────────────────────────

    fn in_path(&self, rel: &str) -> Option<Resolved> {
        let candidate = self.data_in.join(rel);
        if candidate.is_file() {
            return Some(Resolved {
                src: candidate,
                locale: None,
            });
        }
        if let Some(resolved) = resolve_case_insensitive(&candidate).filter(|p| p.is_file()) {
            return Some(Resolved {
                src: resolved,
                locale: None,
            });
        }
        // Fall back to the locale data dirs — mirrors runtime
        // `SbFile::open` alternate-path lookup so files that ship only
        // under `<root>/<lcid>/Data/...` (e.g. `Text/Level.res`) still
        // resolve.  First hit wins.
        for alt in &self.locale_data_dirs {
            let alt_candidate = alt.data_dir.join(rel);
            if alt_candidate.is_file() {
                return Some(Resolved {
                    src: alt_candidate,
                    locale: Some(alt.iso),
                });
            }
            if let Some(resolved) = resolve_case_insensitive(&alt_candidate).filter(|p| p.is_file())
            {
                return Some(Resolved {
                    src: resolved,
                    locale: Some(alt.iso),
                });
            }
        }
        None
    }

    /// Compute the output path for a relative path.  When `locale` is
    /// `Some(iso)` the output lives under `<output>/<iso>/Data/<rel>`
    /// instead of the base `<output>/Data/<rel>`, matching the locale
    /// subtree layout used by the original datadirs.
    fn out_path(&self, rel: &str, locale: Option<&str>) -> PathBuf {
        match locale {
            None => self.data_out.join(rel),
            Some(iso) => {
                let output_root = self
                    .data_out
                    .parent()
                    .expect("data_out always has a parent (the output root)");
                output_root.join(iso).join("Data").join(rel)
            }
        }
    }

    fn exists(&self, rel: &str) -> bool {
        self.in_path(rel).is_some()
    }

    /// Dispatch on extension. Unknown extensions are a hard error so we
    /// never silently drop data we don't know how to handle.
    fn convert_rel(&mut self, rel: &str) -> Result<()> {
        let Some(resolved) = self.in_path(rel) else {
            tracing::warn!("missing: {}", rel);
            self.missing += 1;
            return Ok(());
        };
        let Resolved { src, locale } = resolved;

        let ext = src
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            // ── Structured → JSON ────────────────────────────────────
            "cpf" => {
                let dst = self.out_path(&format!("{rel}.json"), locale);
                convert_cpf(&src, &dst)?;
                self.converted += 1;
            }
            "red" => {
                let dst = self.out_path(&format!("{rel}.json"), locale);
                convert_red(&src, &dst)?;
                self.converted += 1;
            }
            "rhp" => {
                let dst = self.out_path(&format!("{rel}.json"), locale);
                convert_rhp(&src, &dst)?;
                self.converted += 1;
            }
            "rhm" => {
                let dst = self.out_path(&format!("{rel}.json"), locale);
                let beggar = self.beggar_civ_indices.clone();
                convert_rhm(&src, &dst, &move |idx| beggar.contains(&idx))?;
                self.converted += 1;
            }
            "scb" => {
                let dst = self.out_path(&format!("{rel}.json"), locale);
                convert_scb(&src, &dst)?;
                self.converted += 1;
            }
            "rhs" => {
                let dst_dir = self.out_path(&format!("{rel}.d"), locale);
                self.convert_rhs_to_dir(&src, &dst_dir)
                    .with_context(|| format!("converting {rel}"))?;
                self.converted += 1;
            }

            // ── Bundles → directory of JSON + PNGs ────────────────────
            "res" => {
                let dst_dir = self.out_path(&format!("{rel}.d"), locale);
                convert_res(&src, &dst_dir).with_context(|| format!("converting {rel}"))?;
                self.converted += 1;
            }
            "pak" => {
                let dst_dir = self.out_path(&format!("{rel}.d"), locale);
                convert_pak(&src, &dst_dir).with_context(|| format!("converting {rel}"))?;
                self.converted += 1;
            }
            // ── Bitmaps → PNG ─────────────────────────────────────────
            // Terrain background (`.map`) and minimap (`.min`) files use
            // `SBPictureSixteen` on disk — the same 16-bit compressed
            // picture format consumed at runtime via
            // `Picture::load_sixteen_from_stream`.  Decode once and
            // re-encode to PNG so the shipped datadir is self-describing.
            "map" | "min" => {
                let dst = self.out_path(&format!("{rel}.png"), locale);
                convert_sixteen_picture_to_png(&src, &dst)
                    .with_context(|| format!("converting {rel}"))?;
                self.converted += 1;
            }

            // ── Fonts: copy verbatim until a parser lands ─────────────
            //
            // `.bfn` / `.tfn` / `.fnt` are the small bitmap/TrueType
            // fonts shipped with the game.  The runtime still loads
            // them in their raw form, so round-trip them through the
            // datadir unchanged — full JSON/PNG extraction needs a
            // font parser that we haven't ported.
            "bfn" | "tfn" | "fnt" => {
                self.copy_raw(rel, &src, locale)?;
            }

            // ── Standard formats: keep as-is ──────────────────────────
            "wav" | "ogg" => {
                self.copy_raw(rel, &src, locale)?;
            }

            _ => bail!(
                "unknown extension {ext:?} in {rel}; add a dispatch case or \
                 exclude the file from the reference graph"
            ),
        }
        Ok(())
    }

    fn copy_raw(&mut self, rel: &str, src: &Path, locale: Option<&str>) -> Result<()> {
        let dst = self.out_path(rel, locale);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, &dst)
            .with_context(|| format!("copy {} → {}", src.display(), dst.display()))?;
        self.copied += 1;
        Ok(())
    }

    // ── Specialized loaders used during discovery ─────────────────────

    fn load_and_convert_cpf(&mut self, rel: &str) -> Result<ProfileManager> {
        let resolved = self
            .in_path(rel)
            .ok_or_else(|| anyhow!("cpf missing: {rel}"))?;
        let mut file = SbFile::open(&resolved.src.to_string_lossy(), SB_FILE_READ)
            .map_err(|e| anyhow!("open cpf: {e}"))?;
        let mut mgr = ProfileManager::new();
        mgr.load_all_legacy_cpf(&mut file)
            .map_err(|e| anyhow!("parse cpf: {e}"))?;

        let dst = self.out_path(&format!("{rel}.json"), resolved.locale);
        let json = serde_json::to_string_pretty(&mgr)?;
        write_with_parents(&dst, json.as_bytes())?;
        self.converted += 1;
        Ok(mgr)
    }

    fn parse_level(
        &self,
        proto_name: &str,
        mission_name: &str,
    ) -> Result<(LoadedProtoLevel, LoadedMission)> {
        let proto_path = self
            .in_path(&format!("Levels/{proto_name}.rhp"))
            .ok_or_else(|| anyhow!("proto missing: {proto_name}"))?
            .src;
        let mission_path = self
            .in_path(&format!("Levels/{mission_name}.rhm"))
            .ok_or_else(|| anyhow!("mission missing: {mission_name}"))?
            .src;

        let proto_file = SbFile::open(&proto_path.to_string_lossy(), SB_FILE_READ)
            .map_err(|e| anyhow!("open rhp: {e}"))?;
        let mut proto_reader = ChunkReader::new(proto_file);
        let format = {
            let tag = proto_reader
                .peek_next_chunk()
                .map_err(|e| anyhow!("peek proto tag: {e:?}"))?;
            LevelFormat::detect(&tag).map_err(|e| anyhow!("detect format: {e:?}"))?
        };
        let proto = load_proto_level(&mut proto_reader, format)
            .map_err(|e| anyhow!("load proto: {e:?}"))?;

        let mission_file = SbFile::open(&mission_path.to_string_lossy(), SB_FILE_READ)
            .map_err(|e| anyhow!("open rhm: {e}"))?;
        let mut mission_reader = ChunkReader::new(mission_file);
        let beggar = self.beggar_civ_indices.clone();
        let mission = load_mission(&mut mission_reader, format, &|idx| beggar.contains(&idx))
            .map_err(|e| anyhow!("load mission: {e:?}"))?;
        Ok((proto, mission))
    }

    /// Load the shared sprite bank lazily; .rhs conversion is the only
    /// consumer, and datadirs without any referenced characters shouldn't
    /// pay the ~30 MB read.
    fn frame_holder_mut(&mut self) -> Result<&mut FrameHolder> {
        if self.frame_holder.is_none() {
            let parent = self
                .data_in
                .parent()
                .ok_or_else(|| anyhow!("data dir has no parent: {}", self.data_in.display()))?;
            let holder = FrameHolder::from_data_dir(&parent.to_string_lossy())
                .context("loading sprite bank")?;
            self.frame_holder = Some(holder);
        }
        Ok(self.frame_holder.as_mut().unwrap())
    }

    /// Convert a single `.rhs` file into a directory that expands every
    /// referenced sprite frame as a PNG, organised by profile and action.
    fn convert_rhs_to_dir(&mut self, src: &Path, out_dir: &Path) -> Result<()> {
        let (signature, profiles) =
            sprite_scriptor::SpriteScriptor::load_all_profiles(&src.to_string_lossy())
                .map_err(|e| anyhow!("rhs: {e}"))?;

        fs::create_dir_all(out_dir)?;

        // Character `.rhs` files in practice only have one profile. When
        // there's exactly one, drop the redundant profile subdirectory and
        // place actions straight under the `.rhs.d/` root.
        let single_profile = profiles.len() == 1;

        let mut manifest_profiles = Vec::with_capacity(profiles.len());
        for (profile_name, info) in &profiles {
            let profile_dir = if single_profile {
                out_dir.to_path_buf()
            } else {
                out_dir.join(sanitize_path_component(profile_name))
            };
            let mut manifest_rows = Vec::with_capacity(info.scripts.len());

            // Precompute direction index per row (Nth row with a given
            // action_id = facing direction N, per engine convention).
            let mut dir_of_row = vec![0u16; info.scripts.len()];
            let mut dir_counter: std::collections::HashMap<u16, u16> =
                std::collections::HashMap::new();
            for (i, r) in info.scripts.iter().enumerate() {
                let slot = dir_counter.entry(r.action_id).or_insert(0);
                dir_of_row[i] = *slot;
                *slot += 1;
            }

            for (row_idx, row) in info.scripts.iter().enumerate() {
                let action_id = row.action_id as u32;
                let action_label = OrderType::try_from(action_id)
                    .ok()
                    .map(|a| format!("{a:?}"))
                    .unwrap_or_else(|| format!("action_{action_id:04}"));
                let dir = dir_of_row[row_idx];
                // If an action has more than one row, put each direction in
                // its own sub-folder; if it's a single-row action, keep the
                // action folder flat.
                let label_for_dir = if dir_counter[&row.action_id] > 1 {
                    format!("{action_label}/dir_{dir:02}")
                } else {
                    action_label.clone()
                };
                let row_dir = profile_dir.join(&label_for_dir);
                fs::create_dir_all(&row_dir)?;

                let mut frames = Vec::with_capacity(row.frame_ids.len());
                for (frame_idx, &bank_id) in row.frame_ids.iter().enumerate() {
                    let filename = format!("{frame_idx:02}.png");
                    let png_path = row_dir.join(&filename);
                    self.extract_sprite_to_png(bank_id, &png_path)
                        .with_context(|| {
                            format!("sprite {bank_id} for {profile_name}/{label_for_dir}")
                        })?;
                    self.used_sprites.insert(bank_id);
                    frames.push(serde_json::json!({
                        "file": filename,
                        "delay": row.delays.get(frame_idx).copied().unwrap_or(0),
                        "distance": row.distances.get(frame_idx).copied().unwrap_or(0),
                        "offset_x": row.offsets.get(frame_idx).map(|v| v.x).unwrap_or(0.0),
                        "offset_y": row.offsets.get(frame_idx).map(|v| v.y).unwrap_or(0.0),
                        "sound_id": row.sound_ids.get(frame_idx).copied().unwrap_or(0),
                    }));
                }

                manifest_rows.push(serde_json::json!({
                    "action_id": action_id,
                    "action": action_label,
                    "direction": dir,
                    "path": label_for_dir,
                    "action_done": row.action_done,
                    "average_speed": row.average_speed,
                    "hotspot_x": row.hotspot.x,
                    "hotspot_y": row.hotspot.y,
                    "frames": frames,
                }));
            }

            manifest_profiles.push(serde_json::json!({
                "name": profile_name,
                "width": info.size.x,
                "height": info.size.y,
                "center_x": info.center.x,
                "center_y": info.center.y,
                "rows": manifest_rows,
            }));
        }

        let manifest = serde_json::json!({
            "signature": signature,
            "profiles": manifest_profiles,
        });
        fs::write(
            out_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;
        Ok(())
    }

    fn extract_sprite_to_png(&mut self, bank_id: u32, dst: &Path) -> Result<()> {
        let holder = self.frame_holder_mut()?;
        let num = holder.num_sprites();
        if (bank_id as usize) >= num {
            bail!("sprite id {bank_id} out of range (bank has {num})");
        }
        let w = holder.sprite_width(bank_id);
        let h = holder.sprite_height(bank_id);
        if w == 0 || h == 0 {
            // Zero-size entry: still write an empty 1×1 transparent PNG so
            // the manifest reference doesn't dangle.
            return write_png(dst, 1, 1, &[0, 0, 0, 0]);
        }
        write_sprite_png(holder, bank_id, w, h, dst)
    }

    /// Dump bank sprites that no `.rhs` file referenced, so nothing gets
    /// silently dropped. Only runs if the bank was actually loaded.
    fn dump_unused_sprites(&mut self) -> Result<()> {
        let Some(holder) = self.frame_holder.as_ref() else {
            return Ok(());
        };
        let num = holder.num_sprites();
        let unused_ids: Vec<u32> = (0..num as u32)
            .filter(|id| !self.used_sprites.contains(id))
            .collect();
        if unused_ids.is_empty() {
            return Ok(());
        }
        let out_dir = self.data_out.join("_unused_sprites");
        fs::create_dir_all(&out_dir)?;
        tracing::info!(
            "{} sprites were never referenced by any .rhs; dumping to {}",
            unused_ids.len(),
            out_dir.display()
        );
        let mut manifest = Vec::with_capacity(unused_ids.len());
        for id in &unused_ids {
            let w = holder.sprite_width(*id);
            let h = holder.sprite_height(*id);
            let file = format!("{id:06}.png");
            if w > 0 && h > 0 {
                write_sprite_png(holder, *id, w, h, &out_dir.join(&file))?;
            }
            manifest.push(serde_json::json!({
                "id": id,
                "file": if w > 0 && h > 0 { serde_json::Value::String(file) } else { serde_json::Value::Null },
                "width": w,
                "height": h,
            }));
        }
        fs::write(
            out_dir.join("manifest.json"),
            serde_json::to_string_pretty(&serde_json::json!({ "sprites": manifest }))?,
        )?;
        Ok(())
    }
}

fn sanitize_path_component(s: &str) -> String {
    // Profile names come from artist-authored data and may contain anything.
    // Swap out the characters most likely to trip up filesystems; leave
    // spaces alone (existing character files already use them).
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

// ---------------------------------------------------------------------------
// Level reference extraction
// ---------------------------------------------------------------------------

#[derive(Default)]
struct LevelRefs {
    sprite_rhs: BTreeSet<String>,
    map_names: BTreeSet<String>,
    /// Sound-source IDs referenced by each level's `.rhp`. The runtime
    /// maps each ID to a `snd_%03d.wav` file under `Data/Sounds/`.
    sound_wave_ids: BTreeSet<u32>,
}

fn collect_level_refs(proto: &LoadedProtoLevel, mission: &LoadedMission, out: &mut LevelRefs) {
    for p in &proto.patches {
        let n = &p.element_fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.sprite_rhs.insert(n.clone());
        }
    }
    for fx in &proto.animations {
        let n = &fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.sprite_rhs.insert(n.clone());
        }
    }
    if !mission.header.map_filename.is_empty() {
        out.map_names.insert(mission.header.map_filename.clone());
    }
    for p in &mission.mission_patches {
        let n = &p.element_fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.sprite_rhs.insert(n.clone());
        }
    }
    // Targets are Data/Animations sprites resolved via `resolve_rhs_path`
    // (see `engine/level_loading.rs` :1255) — add their `filename` to the
    // referenced-sprite set so the converter emits the `.rhs` / `.bnk`
    // sources they need.
    for t in &mission.targets {
        if !t.filename.is_empty() {
            out.sprite_rhs.insert(t.filename.clone());
        }
    }
    // Sound-source waves: each source's `id` is the sound-bank id the
    // cache composes into `snd_%03d.wav` at runtime. Store the raw id;
    // the converter emits the filename in pass 3.
    for s in &proto.sound_sources {
        if s.id >= 0 {
            out.sound_wave_ids.insert(s.id as u32);
        }
    }
    // `.scb` script-object references: the bytecode quads carry
    // opcode-encoded references to sprite/sound/string IDs, but the
    // parser preserves the raw 8-byte operand tuples without decoding
    // them to typed operands. Following those references needs a VM
    // opcode decoder that hasn't landed yet — leaving as a standalone
    // follow-up so the bulk of today's graph (the direct references
    // above) is already captured.
}

// ---------------------------------------------------------------------------
// Concrete file-format converters
// ---------------------------------------------------------------------------

fn convert_cpf(src: &Path, dst: &Path) -> Result<()> {
    let mut file =
        SbFile::open(&src.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open cpf: {e}"))?;
    let mut mgr = ProfileManager::new();
    mgr.load_all_legacy_cpf(&mut file)
        .map_err(|e| anyhow!("parse cpf: {e}"))?;
    write_json_pretty(dst, &mgr)
}

fn convert_red(src: &Path, dst: &Path) -> Result<()> {
    let desc = res_descr::load(&src.to_string_lossy()).context("loading .red")?;
    write_json_pretty(dst, &desc)
}

fn convert_rhp(src: &Path, dst: &Path) -> Result<()> {
    let file =
        SbFile::open(&src.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open rhp: {e}"))?;
    let mut reader = ChunkReader::new(file);
    let format = {
        let tag = reader
            .peek_next_chunk()
            .map_err(|e| anyhow!("peek: {e:?}"))?;
        LevelFormat::detect(&tag).map_err(|e| anyhow!("format: {e:?}"))?
    };
    let proto = load_proto_level(&mut reader, format).map_err(|e| anyhow!("rhp: {e:?}"))?;
    write_json_pretty(dst, &proto)
}

fn convert_rhm(src: &Path, dst: &Path, is_beggar: &dyn Fn(u32) -> bool) -> Result<()> {
    // The mission file alone doesn't record its format; it must match the
    // sibling proto-level. Probe by trying each known format until one
    // parses cleanly. Fine for a one-shot converter.
    let src_str = src.to_string_lossy().to_string();
    for format in [LevelFormat::Fullgame, LevelFormat::Demo] {
        let file = SbFile::open(&src_str, SB_FILE_READ).map_err(|e| anyhow!("open rhm: {e}"))?;
        let mut reader = ChunkReader::new(file);
        if let Ok(mission) = load_mission(&mut reader, format, is_beggar) {
            return write_json_pretty(dst, &mission);
        }
    }
    bail!("rhm: no known LevelFormat parsed {}", src.display())
}

fn convert_scb(src: &Path, dst: &Path) -> Result<()> {
    let scb = scb::parse_file(src).map_err(|e| anyhow!("scb: {e}"))?;
    write_json_pretty(dst, &scb)
}

fn convert_res(src: &Path, out_dir: &Path) -> Result<()> {
    let mut mgr = ResourceManager::new();
    mgr.attach_resource_file(&src.to_string_lossy())
        .context("resource file parse")?;
    fs::create_dir_all(out_dir)?;

    let mut ids: Vec<_> = mgr.iter_entries().collect();
    ids.sort_by_key(|(id, _)| *id);

    let mut manifest = serde_json::Map::new();
    for (id, type_tag) in ids {
        let tag_str = std::str::from_utf8(&type_tag).unwrap_or("????").trim();
        let mut entry = serde_json::Map::new();
        entry.insert("type".into(), serde_json::Value::String(tag_str.into()));

        if let Some(pics) = mgr.pictures_raw(id) {
            let mut pic_list = Vec::with_capacity(pics.len());
            for (i, pic) in pics.iter().enumerate() {
                pic_list.push(match pic {
                    Some(p) => {
                        let filename = format!("{id:05}_{i:02}.png");
                        write_picture_png(p, &out_dir.join(&filename))?;
                        serde_json::json!({
                            "file": filename,
                            "width": p.width,
                            "height": p.height,
                            "format": format!("{:?}", p.pixel_format),
                        })
                    }
                    None => serde_json::Value::Null,
                });
            }
            entry.insert("pictures".into(), serde_json::Value::Array(pic_list));
            if let Some(m) = mgr.mouse_entry(id) {
                entry.insert(
                    "cursor".into(),
                    serde_json::json!({
                        "hotspot_x": m.hotspot.x,
                        "hotspot_y": m.hotspot.y,
                        "flags": m.flags,
                        "frame_length": m.frame_length,
                    }),
                );
            }
        }
        if let Some(strs) = mgr.strings_raw(id) {
            entry.insert("strings".into(), serde_json::to_value(strs)?);
        }
        if let Some(waves) = mgr.waves_raw(id) {
            entry.insert("waves".into(), serde_json::to_value(waves)?);
        }
        manifest.insert(id.to_string(), serde_json::Value::Object(entry));
    }

    let manifest_path = out_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&serde_json::Value::Object(manifest))?,
    )?;
    Ok(())
}

/// `.pak` files hold a handful of sequential `SBPictureSixteen` images.
/// Loading.pak has 3 (initial/final/height-mask); some level .pak files hold
/// more. Read pictures until EOF and dump each as a PNG.
fn convert_pak(src: &Path, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    let mut file =
        SbFile::open(&src.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open pak: {e}"))?;
    let total = file.get_size();
    let mut entries = Vec::new();
    let mut i = 0usize;
    while file.tell() < total {
        match Picture::load_sixteen_from_stream(&mut file) {
            Ok(pic) => {
                let filename = format!("{i:02}.png");
                write_picture_png(&pic, &out_dir.join(&filename))?;
                entries.push(serde_json::json!({
                    "file": filename,
                    "width": pic.width,
                    "height": pic.height,
                }));
                i += 1;
            }
            Err(e) => bail!("pak picture {i}: {e}"),
        }
    }
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "pictures": entries }))?,
    )?;
    Ok(())
}

fn write_sprite_png(
    holder: &FrameHolder,
    sprite_idx: u32,
    width: u16,
    height: u16,
    dst: &Path,
) -> Result<()> {
    let w = width as usize;
    let h = height as usize;
    let mut pixels = vec![0u16; w * h];
    // 16-bit output, Day variant, no shadow replacement — the raw reference
    // decode of the sprite as shipped.
    holder.uncompress_frame(&mut pixels, w, sprite_idx, SpriteVariant::Day, 0, 16);

    const TRANSPARENT: u16 = 0xF81F; // matches TRANSPARENT_COLOR_16 in frame_holder

    let mut rgba = Vec::with_capacity(w * h * 4);
    for &px in &pixels {
        if px == TRANSPARENT {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let r5 = ((px >> 11) & 0x1F) as u8;
            let g6 = ((px >> 5) & 0x3F) as u8;
            let b5 = (px & 0x1F) as u8;
            rgba.push((r5 << 3) | (r5 >> 2));
            rgba.push((g6 << 2) | (g6 >> 4));
            rgba.push((b5 << 3) | (b5 >> 2));
            rgba.push(0xFF);
        }
    }
    write_png(dst, w as u32, h as u32, &rgba)
}

fn write_picture_png(pic: &Picture, dst: &Path) -> Result<()> {
    let rgba = pic.to_rgba8888(None);
    write_png(dst, pic.width as u32, pic.height as u32, &rgba)
}

/// Decode an `SBPictureSixteen` (`.map` / `.min`) file and re-encode it
/// as a PNG.  The disk format uses `Picture::load_sixteen_from_stream`,
/// which owns the bzip2 decompress of the 16-bit RGB565 payload.
fn convert_sixteen_picture_to_png(src: &Path, dst: &Path) -> Result<()> {
    let mut file = SbFile::open(&src.to_string_lossy(), SB_FILE_READ)
        .map_err(|e| anyhow!("open {}: {e}", src.display()))?;
    let picture = Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decoding {}", src.display()))?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    write_picture_png(&picture, dst)
}

fn write_png(dst: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = fs::File::create(dst).with_context(|| format!("create {}", dst.display()))?;
    let buf = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(buf, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("png header")?;
    writer.write_image_data(rgba).context("png data")?;
    Ok(())
}

fn write_with_parents(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn write_json_pretty<T: serde::Serialize>(dst: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    write_with_parents(dst, json.as_bytes())
}

// ═══════════════════════════════════════════════════════════════════════════
//  Shipping format: one bitcode blob, zstd-compressed at max settings.
// ═══════════════════════════════════════════════════════════════════════════

use robin_rs::level_loader::LoadedLevel;
use robin_rs::shipping_datadir::{RhsData, ShippingDatadir, ShippingSprite, ShippingSpriteBank};

fn convert_shipping(data_in: PathBuf, data_out: &Path, opts: ShippingOpts) -> Result<()> {
    let mut dd = ShippingDatadir::default();
    let mut beggar_ids: BTreeSet<u32> = BTreeSet::new();

    let locale_dirs = detect_locale_data_dirs(&data_in);
    for src in &locale_dirs {
        tracing::info!("Locale data dir [{}]: {}", src.iso, src.data_dir.display());
    }

    // Shipping output is keyed by rel path only (the runtime's
    // `ShippingDatadir` has no locale dimension — each install ships
    // one locale), so we resolve via data_in first and fall back to the
    // locale alt-dirs, matching the runtime `SbFile::open` chain.
    let in_path = |rel: &str| -> Option<PathBuf> {
        let candidate = data_in.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
        if let Some(resolved) = resolve_case_insensitive(&candidate).filter(|p| p.is_file()) {
            return Some(resolved);
        }
        for alt in &locale_dirs {
            let c = alt.data_dir.join(rel);
            if c.is_file() {
                return Some(c);
            }
            if let Some(r) = resolve_case_insensitive(&c).filter(|p| p.is_file()) {
                return Some(r);
            }
        }
        None
    };

    // ── Fixed boot roots ───────────────────────────────────────────────
    // Boot-time resource roots plus the expression/actor text
    // table and loading-screen bundle.
    for rel in [
        "Interface/DEFAULT.RES",
        "Text/actors.res",
        "Text/Level.res",
        "Sounds/Exclamations/actors.res",
    ] {
        if let Some(p) = in_path(rel) {
            let mut mgr = ResourceManager::new();
            mgr.attach_resource_file(&p.to_string_lossy())?;
            if is_interface_path(rel)
                && let Some(q) = opts.interface_image_format.jxl_quality()
            {
                let encoded = mgr.encode_pictures_for_shipping(|pic| {
                    Ok(EncodedPicture::jxl_rgba565_keyed(
                        transcode_picture_to_jxl_rgba_keyed(pic, q)?,
                    ))
                })?;
                tracing::info!(
                    "interface res {rel}: encoded {encoded} pictures as JXL {}",
                    jxl_quality_label(q)
                );
            }
            dd.res_files.insert(rel.into(), mgr);
        }
    }
    if let Some(p) = in_path("Interface/Loading.pak")
        && opts.interface_image_format != InterfaceImageFormat::Raw
    {
        let pictures = read_pak_pictures(&p)?;
        let encoded = encode_interface_pak_pictures(&pictures, opts.interface_image_format)?;
        dd.pak_files.insert("interface/loading.pak".into(), encoded);
    }
    // ── profile.cpf (root index) ───────────────────────────────────────
    let cpf_path =
        in_path("Configuration/profile.cpf").ok_or_else(|| anyhow!("profile.cpf missing"))?;
    let mut cpf = {
        let mut file = SbFile::open(&cpf_path.to_string_lossy(), SB_FILE_READ)
            .map_err(|e| anyhow!("open cpf: {e}"))?;
        let mut mgr = ProfileManager::new();
        mgr.load_all_legacy_cpf(&mut file)
            .map_err(|e| anyhow!("parse cpf: {e}"))?;
        mgr
    };
    for (i, c) in cpf.civilians.iter().enumerate() {
        if c.civilian_type == CivilianType::Beggar {
            beggar_ids.insert(i as u32);
        }
    }

    // Missions → .rhp/.rhm/.scb/.red, also follow level sprite refs.
    let mut required_rhs_profiles: std::collections::BTreeMap<String, BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut map_names: BTreeSet<String> = BTreeSet::new();

    // Demo boot hardcodes the party in `main_entry::detect_demo_mode`, so the
    // trimmer must preserve those character RHS files even when the mission
    // scripts do not reference them directly.
    if in_path("Levels/Dem_Lei_MP.rhm").is_some() {
        add_required_pc_profiles_for_pcs(&mut required_rhs_profiles, &cpf, "RJMT", &in_path);
    }
    if in_path("Levels/Demo_Lin.rhm").is_some() {
        add_required_pc_profiles_for_pcs(&mut required_rhs_profiles, &cpf, "RSABC", &in_path);
    }

    for mp in &cpf.missions {
        if mp.proto_level_filename.is_empty() || mp.mission_filename.is_empty() {
            continue;
        }
        let rhp_rel = format!("Levels/{}.rhp", mp.proto_level_filename);
        let rhm_rel = format!("Levels/{}.rhm", mp.mission_filename);
        let scb_rel = format!("Levels/{}.scb", mp.mission_filename);
        let red_rel = format!("Text/{}", res_descr::red_filename(mp.id));

        let Some(rhp_path) = in_path(&rhp_rel) else {
            tracing::warn!("missing: {}", rhp_rel);
            continue;
        };
        let Some(rhm_path) = in_path(&rhm_rel) else {
            tracing::warn!("missing: {}", rhm_rel);
            continue;
        };

        let (proto, mission) = parse_level_pair(&rhp_path, &rhm_path, &beggar_ids)?;
        // Collect sprite/map refs.
        for p in &proto.patches {
            add_required_animation_rhs_profile(
                &mut required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for fx in &proto.animations {
            add_required_animation_rhs_profile(
                &mut required_rhs_profiles,
                mission.header.ambiance,
                &fx.sprite,
                &in_path,
            );
        }
        if !mission.header.map_filename.is_empty() {
            map_names.insert(mission.header.map_filename.clone());
        }
        for &idx in &mp.required_character_indices {
            if let Some((rel, profile)) =
                existing_character_rhs_for_index(&cpf, idx as usize, &in_path)
            {
                add_required_rhs_rel(&mut required_rhs_profiles, rel, &profile);
            }
        }
        for p in &mission.mission_patches {
            add_required_animation_rhs_profile(
                &mut required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for target in &mission.targets {
            add_required_rhs_rel(
                &mut required_rhs_profiles,
                animation_rhs_rel_existing(mission.header.ambiance, &target.filename, &in_path),
                &target.profile_name,
            );
        }
        for soldier in &mission.soldiers {
            if let Some(profile) = cpf.soldiers.get(soldier.profile_number as usize) {
                add_required_rhs_rel(
                    &mut required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for civilian in &mission.civilians {
            if let Some(profile) = cpf.civilians.get(civilian.profile_number as usize) {
                add_required_rhs_rel(
                    &mut required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for pc in &mission.pcs_to_rescue {
            if let Some((rel, profile)) =
                existing_character_rhs_for_index(&cpf, pc.profile_index as usize, &in_path)
            {
                add_required_rhs_rel(&mut required_rhs_profiles, rel, &profile);
            }
        }
        for bonus in &mission.bonuses {
            if let Some((file, profile)) = bonus_type_to_sprite_asset_for_shipping(bonus.bonus_type)
            {
                add_required_rhs_rel(
                    &mut required_rhs_profiles,
                    format!("Characters/{file}.rhs"),
                    profile,
                );
            }
        }
        if !mission.scrolls.is_empty() {
            add_required_rhs_rel(
                &mut required_rhs_profiles,
                "Characters/BONUS_Parchment.rhs",
                "BONUS Parchemin",
            );
            add_required_rhs_rel(
                &mut required_rhs_profiles,
                "Characters/BONUS_FourLeavedClover.rhs",
                "BONUS Trefle",
            );
        }
        add_common_object_rhs_profiles(&mut required_rhs_profiles);
        add_required_rhs_rel(
            &mut required_rhs_profiles,
            "Characters/Blip00.rhs",
            "Blip 00",
        );

        dd.levels
            .insert(mp.mission_filename.clone(), LoadedLevel { proto, mission });

        if let Some(p) = in_path(&scb_rel) {
            let parsed = scb::parse_file(&p).map_err(|e| anyhow!("scb: {e}"))?;
            dd.scripts.insert(mp.mission_filename.clone(), parsed);
        } else {
            tracing::warn!("missing: {}", scb_rel);
        }
        if let Some(p) = in_path(&red_rel) {
            let desc = res_descr::load(&p.to_string_lossy())?;
            dd.red_files.insert(res_descr::red_filename(mp.id), desc);
        }
    }

    // ── .rhs files needed by the converted missions ────────────────────
    let mut used_sprite_ids: BTreeSet<u32> = BTreeSet::new();
    let mut broad_rhs_sprite_ids: BTreeSet<u32> = BTreeSet::new();
    let mut resolved_rhs_profiles = 0usize;
    for (rel, required_profiles) in &required_rhs_profiles {
        if rel.is_empty() {
            continue;
        }
        if let Some(p) = in_path(rel) {
            let (signature, profiles) =
                sprite_scriptor::SpriteScriptor::load_all_profiles(&p.to_string_lossy())
                    .map_err(|e| anyhow!("rhs {rel}: {e}"))?;
            let all_profiles_required = required_profiles.contains("");
            let mut matched_profiles = BTreeSet::new();
            for (profile_name, info) in &profiles {
                for script in info.scripts.iter() {
                    for &id in &script.frame_ids {
                        broad_rhs_sprite_ids.insert(id);
                        if all_profiles_required || required_profiles.contains(profile_name) {
                            used_sprite_ids.insert(id);
                        }
                    }
                }
                if all_profiles_required || required_profiles.contains(profile_name) {
                    matched_profiles.insert(profile_name.clone());
                    resolved_rhs_profiles += 1;
                }
            }
            for required in required_profiles {
                if !required.is_empty() && !matched_profiles.contains(required) {
                    tracing::warn!("rhs {rel}: missing required profile '{required}'");
                }
            }
            dd.rhs_files.insert(
                rel.clone(),
                RhsData {
                    signature,
                    profiles,
                },
            );
        } else {
            tracing::warn!("missing: {}", rel);
        }
    }

    // ── Sprite bank (always, since the engine needs it) ────────────────
    let parent = data_in
        .parent()
        .ok_or_else(|| anyhow!("data dir has no parent"))?;
    let holder =
        FrameHolder::from_data_dir(&parent.to_string_lossy()).context("loading sprite bank")?;
    // Keep the bank indices stable, but omit packed payloads for slots that
    // no mission-start RHS profile references. Missing proto-level RHS refs
    // are absent from the demo datadir already, so they do not contribute
    // renderable sprite IDs here.
    let sprites: Vec<Option<ShippingSprite>> = holder
        .sprites()
        .iter()
        .enumerate()
        .map(|(idx, s)| {
            if used_sprite_ids.contains(&(idx as u32)) {
                Some(ShippingSprite {
                    width: s.width,
                    height: s.height,
                    dictionary_index: s.dictionary_index,
                    packed_data: s.packed_data.clone().unwrap_or_default(),
                })
            } else {
                None
            }
        })
        .collect();
    tracing::info!(
        "sprite bank: keeping {} / {} sprites ({} required RHS profiles, {} broad RHS refs)",
        used_sprite_ids.len(),
        holder.sprites().len(),
        resolved_rhs_profiles,
        broad_rhs_sprite_ids.len(),
    );
    dd.sprite_bank = Some(ShippingSpriteBank {
        signature: holder.signature(),
        dictionaries: holder.dictionaries().to_vec(),
        sprites,
    });

    // Bake the `import_beam_mes` post-processing into the shipping
    // profile table.  Without this, runtime loaders that consume
    // `dd.profiles` see empty `required_actions` / zero
    // `number_of_beam_mes` — breaking briefing-UI glyphs and
    // auto-gang-selection (see
    // `crates/robin_rs/src/main_entry.rs::load_profiles` for the
    // non-shipping equivalent).
    if let Some(level_dir) = in_path("Levels").map(|p| p.to_string_lossy().into_owned()) {
        cpf.import_beam_mes(&level_dir);
    } else {
        tracing::warn!(
            "convert_shipping: no Levels/ directory found; shipping profile will lack beam-me data"
        );
    }
    dd.profiles = Some(cpf);

    // ── Raw blobs (not-yet-parsed file types) ──────────────────────────
    // Terrain bitmaps referenced by missions; try each ambience subfolder.
    // `.map` files can optionally be re-encoded as JXL to shrink the blob.
    for map in &map_names {
        for sub in ["Day", "Night", "Fog"] {
            for ext in [".map", ".min"] {
                let rel = format!("Levels/{sub}/{map}{ext}");
                let Some(p) = in_path(&rel) else {
                    continue;
                };
                let bytes = match (ext, opts.map_format) {
                    (".map", MapFormat::JxlLossless) => transcode_sixteen_to_jxl(&p, None)?,
                    (".map", MapFormat::JxlQ90) => transcode_sixteen_to_jxl(&p, Some(90))?,
                    (".map", MapFormat::JxlQ85) => transcode_sixteen_to_jxl(&p, Some(85))?,
                    (".map", MapFormat::JxlQ80) => transcode_sixteen_to_jxl(&p, Some(80))?,
                    // Raw mode: decode the bzip2-packed SBPictureSixteen
                    // and re-encode with `SixteenPacking::None`.  Required
                    // for wasm builds (which stub out the bzip2 decoder)
                    // and harmless for native — outer shipping zstd-22
                    // catches the RGB565 redundancy that bzip2 was
                    // removing, so the blob doesn't bloat.
                    (".map", MapFormat::Raw) => transcode_sixteen_drop_bzip(&p)?,
                    (".min", _) => transcode_sixteen_drop_bzip(&p)?,
                    _ => fs::read(&p)?,
                };
                dd.raw.insert(rel.to_ascii_lowercase(), bytes);
            }
        }
    }
    // Bundle the small-file types the engine opens by exact path — these
    // are the items that would otherwise fan out to hundreds of tiny HTTP
    // requests on wasm and a bunch of syscalls on native.  We deliberately
    // *don't* bundle large files (audio, terrain bitmaps already handled
    // above, cinematics) so the shipping blob stays compact.
    //
    // Keyed by the path the engine passes to `SbFile::open` minus the
    // `Data/` prefix, which matches `asset_fs::bundle_key`.
    const SMALL_FILE_EXTS: &[&str] = &[
        // Fonts
        "bfn", "tfn", "fnt", // Menu / cursor / interface configuration
        "cfg", "ini", // Resource bundles (text tables, cursors, loading screens)
        "res", "pak", "red", // Binary game data
        "cpf", "rhp", "rhm", "rhs", "scb",
    ];
    walk_and_bundle_small(
        &mut dd,
        &data_in,
        &data_in,
        SMALL_FILE_EXTS,
        opts.interface_image_format,
    )?;
    for alt in &locale_dirs {
        walk_and_bundle_small(
            &mut dd,
            &alt.data_dir,
            &alt.data_dir,
            SMALL_FILE_EXTS,
            opts.interface_image_format,
        )?;
    }
    // Do not bundle the legacy `.bks` / `.dic` sprite-bank files here.
    // Shipping output already contains a parsed `ShippingSpriteBank`, and
    // `FrameHolder::initialize_sprite_bank_with_progress` short-circuits to
    // it before attempting loose-file I/O. Keeping the legacy bank in `raw`
    // nearly doubles the sprite payload in wasm shipping blobs.

    // Serialize + compress with the configured window log.
    let out_file = data_out.join("datadir.bin");
    let blob = bitcode::serialize(&dd).map_err(|e| anyhow!("bitcode encode: {e:?}"))?;
    let compressed =
        robin_rs::shipping_datadir::zstd_compress_with_window(&blob, opts.zstd_window_log)?;
    fs::write(&out_file, compressed).with_context(|| format!("write {}", out_file.display()))?;
    tracing::info!(
        "wrote {} (windowLog={}, map={:?})",
        out_file.display(),
        opts.zstd_window_log,
        opts.map_format
    );
    Ok(())
}

/// Decode an `SBPictureSixteen` (`.map`) file and re-encode it as JXL via
/// the `cjxl` CLI. `quality = None` → lossless modular (`-d 0 --modular=1`);
/// `Some(q)` → VarDCT at quality `q`. Use effort 7: effort 9 did not
/// produce a meaningful size win for this content and is much slower.
///
/// Maps are fully opaque, so we feed cjxl an RGB-only (3-channel) PNG.
/// That makes the resulting JXL have zero extra channels, which keeps
/// the runtime decoder's pixel-format setup trivial (no need to allocate
/// a discard buffer for an alpha extra-channel that's always 255).
fn transcode_sixteen_to_jxl(src: &Path, quality: Option<u8>) -> Result<Vec<u8>> {
    let mut file = SbFile::open(&src.to_string_lossy(), SB_FILE_READ)
        .map_err(|e| anyhow!("open {}: {e}", src.display()))?;
    let pic = Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decode {}", src.display()))?;
    transcode_picture_to_jxl(&pic, quality)
}

fn is_interface_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized == "interface/default.res"
        || normalized == "interface/loading.pak"
        || normalized.starts_with("interface/")
        || normalized.contains("/data/interface/")
}

fn encode_interface_pak_pictures(
    pictures: &[Picture],
    format: InterfaceImageFormat,
) -> Result<Vec<EncodedPicture>> {
    let Some(q) = format.jxl_quality() else {
        bail!("raw interface pak pictures should stay in dd.raw, not dd.pak_files");
    };
    pictures
        .iter()
        .enumerate()
        .map(|(idx, pic)| {
            Ok(EncodedPicture::jxl_rgba565_keyed(
                transcode_picture_to_jxl_rgba_keyed(pic, q).with_context(|| {
                    format!(
                        "interface pak picture {idx}: encode JXL {}",
                        jxl_quality_label(q)
                    )
                })?,
            ))
        })
        .collect()
}

fn jxl_quality_label(quality: Option<u8>) -> String {
    quality
        .map(|q| format!("q{q}"))
        .unwrap_or_else(|| "lossless".to_string())
}

fn transcode_picture_to_jxl_rgba_keyed(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    use robin_rs::frame_holder::TRANSPARENT_COLOR_16;
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let rgba = pic.to_rgba8888(Some(TRANSPARENT_COLOR_16));

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, pic.width as u32, pic.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().context("png header")?;
        w.write_image_data(&rgba).context("png data")?;
    }

    let mut cmd = Command::new("cjxl");
    if let Some(q) = quality {
        cmd.args(["-q", &q.to_string(), "-e", "9", "-", "-"]);
    } else {
        cmd.args(["-d", "0", "--modular=1", "-e", "9", "-", "-"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn cjxl (is it installed?)")?;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&png_bytes)
        .context("write PNG to cjxl")?;
    let out = child.wait_with_output().context("cjxl wait")?;
    if !out.status.success() {
        bail!("cjxl failed: exit {}", out.status);
    }
    Ok(out.stdout)
}

fn transcode_picture_to_jxl(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let rgb = picture_to_rgb888(pic)?;

    // Cjxl takes PNG on stdin → JXL on stdout. Feed a minimal RGB PNG.
    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, pic.width as u32, pic.height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().context("png header")?;
        w.write_image_data(&rgb).context("png data")?;
    }

    let mut cmd = Command::new("cjxl");
    if let Some(q) = quality {
        cmd.args(["-q", &q.to_string(), "-e", "7", "-", "-"]);
    } else {
        cmd.args(["-d", "0", "--modular=1", "-e", "7", "-", "-"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn cjxl (is it installed?)")?;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&png_bytes)
        .context("write PNG to cjxl")?;
    let out = child.wait_with_output().context("cjxl wait")?;
    if !out.status.success() {
        bail!("cjxl failed: exit {}", out.status);
    }
    Ok(out.stdout)
}

fn picture_to_rgb888(pic: &Picture) -> Result<Vec<u8>> {
    use robin_rs::picture::PixelFormat;

    let n = pic.width as usize * pic.height as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    match pic.pixel_format {
        PixelFormat::Rgb16 => {
            if pic.data.len() < n * 2 {
                bail!("RGB565 picture data is truncated");
            }
            for i in 0..n {
                let lo = pic.data[i * 2] as u16;
                let hi = pic.data[i * 2 + 1] as u16;
                let px = lo | (hi << 8);
                let r5 = ((px >> 11) & 0x1F) as u8;
                let g6 = ((px >> 5) & 0x3F) as u8;
                let b5 = (px & 0x1F) as u8;
                rgb.push((r5 << 3) | (r5 >> 2));
                rgb.push((g6 << 2) | (g6 >> 4));
                rgb.push((b5 << 3) | (b5 >> 2));
            }
        }
        _ => {
            let rgba = pic.to_rgba8888(None);
            for px in rgba.chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
            }
        }
    }
    Ok(rgb)
}

/// Recursively walk `src`, bundling every file whose extension is in
/// `exts` into `dd.raw`, keyed by the lowercased path relative to `root`.
/// The key scheme matches `asset_fs::bundle_key`, so runtime callers with
/// any casing hit the same entry.  Existing entries are preserved.
///
/// `.pak` and `.res` files containing inner `SBPictureSixteen` blobs get
/// transcoded so any `Bzip` packing becomes `None` — the `bzip2` C
/// library doesn't build for `wasm32-unknown-emscripten`, and the outer
/// shipping zstd-22 catches the cross-picture redundancy more effectively
/// than per-picture bzip2 anyway.
fn walk_and_bundle_small(
    dd: &mut ShippingDatadir,
    root: &Path,
    src: &Path,
    exts: &[&str],
    interface_image_format: InterfaceImageFormat,
) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_and_bundle_small(dd, root, &path, exts, interface_image_format)?;
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let Some(ext) = ext else { continue };
        if !exts.iter().any(|e| *e == ext) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if dd.raw.contains_key(&rel) {
            continue;
        }
        if interface_image_format != InterfaceImageFormat::Raw
            && is_interface_path(&rel)
            && matches!(ext.as_str(), "res" | "pak")
        {
            if ext == "pak" {
                let pictures = read_pak_pictures(&path)?;
                dd.pak_files.insert(
                    rel.clone(),
                    encode_interface_pak_pictures(&pictures, interface_image_format)?,
                );
            }
            continue;
        }
        let bytes = match ext.as_str() {
            "pak" => transcode_pak_drop_bzip(&path)
                .with_context(|| format!("transcode pak {}: keeping raw bytes", path.display()))?,
            "res" => transcode_res_drop_bzip(&path)
                .with_context(|| format!("transcode res {}: keeping raw bytes", path.display()))?,
            "bfn" => transcode_bfn_drop_bzip(&path)
                .with_context(|| format!("transcode bfn {}", path.display()))?,
            _ => fs::read(&path)
                .with_context(|| format!("walk_and_bundle_small: read {}", path.display()))?,
        };
        dd.raw.insert(rel, bytes);
    }
    Ok(())
}

/// `.pak` files are a back-to-back sequence of `SBPictureSixteen` blobs —
/// reuse `read_pak_pictures` for the parse and `Picture::write_sixteen_to_bytes`
/// for the write-back, choosing `SixteenPacking::None` so the bzip2-only
/// inner compression is gone.  Outer shipping zstd-22 then catches the
/// cross-picture redundancy.
fn transcode_pak_drop_bzip(path: &Path) -> Result<Vec<u8>> {
    use robin_rs::picture::SixteenPacking;
    let pics = read_pak_pictures(path)?;
    let mut out = Vec::new();
    for pic in &pics {
        out.extend(pic.write_sixteen_to_bytes(SixteenPacking::None)?);
    }
    Ok(out)
}

/// `.min` / `.map` bitmaps: a single `SBPictureSixteen`.  Decode the
/// bzip2-packed RGB565 payload and write it back with
/// `SixteenPacking::None` so wasm (which stubs out the bzip2 decoder)
/// can read the image straight from the shipping datadir.
fn transcode_sixteen_drop_bzip(path: &Path) -> Result<Vec<u8>> {
    use robin_rs::picture::SixteenPacking;
    let mut file = SbFile::open(&path.to_string_lossy(), SB_FILE_READ)
        .map_err(|e| anyhow!("open {}: {e}", path.display()))?;
    let pic = Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decoding {}", path.display()))?;
    pic.write_sixteen_to_bytes(SixteenPacking::None)
        .with_context(|| format!("re-encoding {}", path.display()))
}

/// `.bfn` native font files: a fixed header + `char_number` character
/// records + two back-to-back `SBPictureSixteen` pictures (glyph atlas
/// plus alpha mask). The picture payloads ship `SixteenPacking::Bzip`
/// on the original retail discs, so we decode them now and re-emit the
/// whole file with `SixteenPacking::None`. Matches the
/// `SBNativeFont::Load` format — see `crate::native_font` for the
/// reader-side layout.
fn transcode_bfn_drop_bzip(path: &Path) -> Result<Vec<u8>> {
    use robin_rs::picture::SixteenPacking;
    use std::io::Write;

    const TAG_LEN: usize = 6;
    const FONT_NAME_LEN: usize = 32;

    let buf = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if buf.len() < TAG_LEN + 4 + FONT_NAME_LEN + 24 {
        bail!("bfn file truncated before picture payloads");
    }
    if &buf[..TAG_LEN] != b"SBFONT" {
        bail!(
            "not a SBFONT file ({:?})",
            std::str::from_utf8(&buf[..TAG_LEN]).unwrap_or("???")
        );
    }
    let version = u32::from_le_bytes(buf[TAG_LEN..TAG_LEN + 4].try_into().unwrap());

    // Fixed header layout — see native_font.rs::NativeFont::load:
    //   tag (6) | version (4) | name (32) | flags (4) | styles (4) |
    //   height (4) | char_cell_width (4) | baseline (4) | char_number (4)
    //   | (version >= 0x0200: extra_spacing (4))
    //   | char_number * (u16 code, u32 start, u32 width, i32 pre, i32 post)
    let char_number_off = TAG_LEN + 4 + FONT_NAME_LEN + 4 + 4 + 4 + 4 + 4;
    let char_number = u32::from_le_bytes(
        buf[char_number_off..char_number_off + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    let mut pictures_start = char_number_off + 4;
    if version >= 0x0200 {
        pictures_start += 4; // extra_spacing
    }
    pictures_start += char_number * 18; // each char record is 2+4+4+4+4
    if pictures_start > buf.len() {
        bail!("bfn picture start offset out of bounds");
    }

    // Decode both SBPictureSixteen payloads via the existing
    // `load_sixteen_from_bytes` helper (owns the bzip2 decode).
    let remaining = &buf[pictures_start..];
    let glyph = Picture::load_sixteen_from_bytes(remaining)
        .with_context(|| format!("{}: glyph picture", path.display()))?;
    let glyph_size = picture_sixteen_size_on_disk(remaining)?;
    let alpha = Picture::load_sixteen_from_bytes(&remaining[glyph_size..])
        .with_context(|| format!("{}: alpha picture", path.display()))?;

    // Rewrite: keep the header up to the pictures verbatim, then
    // append the two pictures with `SixteenPacking::None`.
    let header = &buf[..pictures_start];
    let mut out = Vec::with_capacity(header.len() + glyph.data.len() + alpha.data.len() + 32);
    out.write_all(header)?;
    out.write_all(&glyph.write_sixteen_to_bytes(SixteenPacking::None)?)?;
    out.write_all(&alpha.write_sixteen_to_bytes(SixteenPacking::None)?)?;
    Ok(out)
}

/// Return the number of bytes a packed `SBPictureSixteen` occupies at
/// the start of `bytes`: 12 B header + `packed_size` payload.  The
/// header layout matches [`Picture::load_sixteen_from_bytes`]:
/// `u16 width, u16 height, u32 packing_raw, u32 packed_size`.
fn picture_sixteen_size_on_disk(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < 12 {
        bail!("sixteen picture header truncated");
    }
    let packed_size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    Ok(12 + packed_size)
}

/// `.res` files: parse via `ResourceManager::attach_resource_file` (the
/// existing chunk reader) and serialise back via `write_to_res_bytes`
/// with `SixteenPacking::None`.  Per-resource `flags` aren't preserved by
/// the reader, so the rewritten file emits `0` for them — this matches
/// the runtime, which never reads back the flags field.
fn transcode_res_drop_bzip(path: &Path) -> Result<Vec<u8>> {
    use robin_rs::picture::SixteenPacking;
    let mut rm = ResourceManager::new();
    rm.attach_resource_file(&path.to_string_lossy())?;
    rm.write_to_res_bytes(SixteenPacking::None)
}

fn add_required_rhs_rel(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    rel: impl Into<String>,
    profile: &str,
) {
    if profile.is_empty() {
        return;
    }
    required
        .entry(rel.into())
        .or_default()
        .insert(profile.into());
}

fn add_required_animation_rhs_profile(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    ambiance: u32,
    sprite: &robin_rs::level_loader::RawSpriteRef,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    if sprite.frame_profile_name.is_empty() || sprite.profile_name.is_empty() {
        return;
    }
    add_required_rhs_rel(
        required,
        animation_rhs_rel_existing(ambiance, &sprite.frame_profile_name, in_path),
        &sprite.profile_name,
    );
}

fn animation_rhs_rel_existing(
    ambiance: u32,
    file: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> String {
    let dir = match ambiance {
        1 => "Fog",
        2 => "Night",
        3 => "Attack",
        16 => "Custom1",
        32 => "Custom2",
        64 => "Custom3",
        128 => "Custom4",
        _ => "Day",
    };
    let primary = format!("Animations/{dir}/{file}.rhs");
    if in_path(&primary).is_some() {
        return primary;
    }
    if dir != "Day" {
        let day = format!("Animations/Day/{file}.rhs");
        if in_path(&day).is_some() {
            return day;
        }
    }
    let base = format!("Animations/{file}.rhs");
    if in_path(&base).is_some() {
        return base;
    }
    primary
}

fn existing_character_rhs_for_index(
    profiles: &ProfileManager,
    index: usize,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Option<(String, String)> {
    let profile = profiles.characters.get(index)?;
    let rel = format!("Characters/{}.rhs", profile.filename);
    if in_path(&rel).is_some() {
        return Some((rel, profile.profile_name.clone()));
    }
    existing_character_rhs_for_profile_name(profiles, &profile.profile_name, in_path)
        .or_else(|| Some((rel, profile.profile_name.clone())))
}

fn existing_character_rhs_for_profile_name(
    profiles: &ProfileManager,
    profile_name: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Option<(String, String)> {
    profiles
        .characters
        .iter()
        .filter(|profile| profile.profile_name == profile_name)
        .find_map(|profile| {
            let rel = format!("Characters/{}.rhs", profile.filename);
            in_path(&rel)
                .is_some()
                .then(|| (rel, profile.profile_name.clone()))
        })
}

fn add_required_pc_profiles_for_pcs(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    pcs: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    for profile_name in pcs.chars().filter_map(pc_code_profile_name) {
        if let Some((rel, profile)) =
            existing_character_rhs_for_profile_name(profiles, profile_name, in_path)
        {
            add_required_rhs_rel(required, rel, &profile);
        } else {
            tracing::warn!("demo PC profile '{}' has no shipped RHS", profile_name);
        }
    }
}

fn pc_code_profile_name(code: char) -> Option<&'static str> {
    match code.to_ascii_uppercase() {
        'R' => Some("Robin des bois"),
        'J' => Some("Petit Jean"),
        'T' => Some("Frere Tuck"),
        'S' => Some("Stutely"),
        'W' => Some("Will Ecarlate"),
        'M' => Some("Lady Marianne"),
        'A' => Some("Paysan A"),
        'B' => Some("Paysan B"),
        'C' => Some("Paysan C"),
        _ => {
            tracing::warn!("unknown demo PC code '{}'", code);
            None
        }
    }
}

fn bonus_type_to_sprite_asset_for_shipping(
    raw_bonus_type: u16,
) -> Option<(&'static str, &'static str)> {
    match raw_bonus_type {
        0 => Some(("BONUS_Arrows", "BONUS Fleches")),
        1 => Some(("BONUS_Stones", "BONUS Cailloux")),
        2 => Some(("BONUS_Apples", "BONUS Pommes")),
        3 => Some(("BONUS_Ale", "BONUS Ale")),
        4 => Some(("BONUS_LegOfLamb", "BONUS Gigots")),
        5 => Some(("BONUS_Plants", "BONUS Plantes")),
        6 => Some(("BONUS_Nets", "BONUS Filets")),
        7 => Some(("BONUS_WaspsNest", "BONUS Guepes")),
        8 => Some(("BONUS_MoneyBag", "BONUS Bourses d'argent")),
        9 => Some(("BONUS_GoldBagsRansom", "BONUS Sac d'or rancon")),
        10 => Some(("BONUS_FourLeavedClover", "BONUS Trefle")),
        11 => Some(("BONUS_Shield", "Shield")),
        12 => Some(("RELIC_Ampulla", "Huile")),
        13 => Some(("RELIC_Spoon", "Cuillere")),
        14 => Some(("RELIC_Crown", "Couronne")),
        15 => Some(("RELIC_Stamp", "Sceau")),
        16 => Some(("RELIC_Sceptre", "Sceptre")),
        17 => Some(("RELIC_Book", "Registre")),
        18 => Some(("RELIC_Sword", "Epee")),
        _ => None,
    }
}

fn add_common_object_rhs_profiles(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
) {
    for (file, profile) in [
        ("ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
        ("ACCESSORIES_Stone", "ACCESSOIRES Cailloux"),
        ("ACCESSORIES_Ale", "ACCESSOIRES Ale"),
        ("ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
        ("ACCESSORIES_MoneyBag", "ACCESSOIRES Bourse d'argent"),
        ("ACCESSORIES_Wasp", "ACCESSOIRES Guepes"),
        ("ACCESSORIES_Coat", "Manteau"),
        ("ACCESSORIES_Net", "ACCESSOIRES Filet"),
        ("ACCESSORIES_Coin", "ACCESSOIRES Piece d'or"),
        ("ACCESSORIES_WaspSting", "Guepe"),
        ("BONUS_Nets", "BONUS Filets"),
        ("BONUS_WaspsNest", "BONUS Guepes"),
    ] {
        add_required_rhs_rel(required, format!("Characters/{file}.rhs"), profile);
    }
}

fn parse_level_pair(
    rhp: &Path,
    rhm: &Path,
    beggar_ids: &BTreeSet<u32>,
) -> Result<(LoadedProtoLevel, LoadedMission)> {
    let file =
        SbFile::open(&rhp.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open rhp: {e}"))?;
    let mut reader = ChunkReader::new(file);
    let format = {
        let tag = reader
            .peek_next_chunk()
            .map_err(|e| anyhow!("peek: {e:?}"))?;
        LevelFormat::detect(&tag).map_err(|e| anyhow!("format: {e:?}"))?
    };
    let proto = load_proto_level(&mut reader, format).map_err(|e| anyhow!("rhp: {e:?}"))?;

    let file =
        SbFile::open(&rhm.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open rhm: {e}"))?;
    let mut reader = ChunkReader::new(file);
    let mission = load_mission(&mut reader, format, &|idx| beggar_ids.contains(&idx))
        .map_err(|e| anyhow!("rhm: {e:?}"))?;
    Ok((proto, mission))
}

fn read_pak_pictures(src: &Path) -> Result<Vec<Picture>> {
    let mut file =
        SbFile::open(&src.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open pak: {e}"))?;
    let total = file.get_size();
    let mut pics = Vec::new();
    while file.tell() < total {
        pics.push(Picture::load_sixteen_from_stream(&mut file).context("pak picture")?);
    }
    Ok(pics)
}
