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
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use robin_assets::frame_holder::{FrameDictionary, FrameHolder, SpriteVariant, UNMAPPED_DICT};
use robin_assets::picture::Picture;
use robin_assets::res_descr;
use robin_assets::resource_manager::{EncodedPicture, ResourceManager};
use robin_assets::scb;
use robin_engine::level_data::{
    ChunkReader, LevelFormat, LoadedMission, LoadedProtoLevel, load_mission, load_proto_level,
};
use robin_engine::order::OrderType;
use robin_engine::profiles::{Action, CivilianType, ProfileManager};
use robin_engine::sbfile::{SB_FILE_READ, SbFile, resolve_case_insensitive};
use robin_engine::sprite_script;
use robin_rs::main_entry::{FALLBACK_LOCALE_FOLDER, LANGUAGE_FOLDERS};

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
    #[arg(long, conflicts_with = "resume")]
    force: bool,

    /// Shipping: keep a partial output directory and reuse every existing
    /// content chunk whose decoded payload exactly matches this conversion.
    /// Useful after an interrupted max-compression run.
    #[arg(long, conflicts_with = "force")]
    resume: bool,

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

    /// Shipping audio representation. `opus` is intended for browser
    /// artifacts; native/loose datadirs keep their source formats.
    #[arg(long, value_enum, default_value_t = AudioFormat::Source)]
    audio_format: AudioFormat,

    /// Shipping: reorder every sprite dictionary by descending tile-index
    /// frequency and rewrite all VQ sprite indices to match. Invisible to
    /// the decoder (a consistent permutation), but the ranked index streams
    /// compress ~5% smaller under zstd (docs/COMPRESSION.md, 2026-08-28).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    rank_dictionaries: bool,
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
    /// Shipping transcodes `.map` files to JXL quality 70.
    JxlQ70,
}

impl MapFormat {
    /// `None` = keep raw; `Some(None)` = lossless JXL; `Some(Some(q))` = lossy.
    fn jxl_quality(self) -> Option<Option<u8>> {
        match self {
            Self::Raw => None,
            Self::JxlLossless => Some(None),
            Self::JxlQ90 => Some(Some(90)),
            Self::JxlQ85 => Some(Some(85)),
            Self::JxlQ80 => Some(Some(80)),
            Self::JxlQ70 => Some(Some(70)),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum AudioFormat {
    /// Preserve source WAV or Ogg/Vorbis bytes.
    Source,
    /// Transcode all selected audio to deterministic Ogg/Opus.
    Opus,
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
    /// Encode interface resource pictures as JXL quality 70.
    JxlQ70,
}

impl InterfaceImageFormat {
    fn jxl_quality(self) -> Option<Option<u8>> {
        match self {
            Self::Raw => None,
            Self::JxlLossless => Some(None),
            Self::JxlQ90 => Some(Some(90)),
            Self::JxlQ85 => Some(Some(85)),
            Self::JxlQ80 => Some(Some(80)),
            Self::JxlQ70 => Some(Some(70)),
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
        } else if !args.resume {
            bail!(
                "output exists (pass --force to overwrite or --resume to validate and reuse chunks): {}",
                args.output.display()
            );
        } else {
            tracing::info!(output = %args.output.display(), "resuming shipping conversion");
        }
    }
    fs::create_dir_all(&args.output)?;

    let data_in = find_data_dir(&args.input)?;
    let data_out = args.output.join("Data");
    fs::create_dir_all(&data_out)?;

    match args.format {
        OutFormat::Hackable if args.resume => {
            bail!("--resume is supported only with --format shipping")
        }
        OutFormat::Hackable => Converter::new(data_in, data_out).run(),
        OutFormat::Shipping => convert_shipping(
            data_in,
            &data_out,
            ShippingOpts {
                map_format: args.map_format,
                interface_image_format: args.interface_image_format,
                audio_format: args.audio_format,
                zstd_window_log: args.zstd_window_log,
                resume: args.resume,
                rank_dictionaries: args.rank_dictionaries,
            },
        ),
    }
}

#[derive(Debug, Clone, Copy)]
struct ShippingOpts {
    map_format: MapFormat,
    interface_image_format: InterfaceImageFormat,
    audio_format: AudioFormat,
    zstd_window_log: u32,
    resume: bool,
    rank_dictionaries: bool,
}

/// Locate the game data directory inside the input folder.
/// The original datadir capitalization varies (`DATA/` in demos, `Data/` in fullgame).
/// Count how often every dictionary entry is referenced across the whole
/// bank and derive an old→new index map per dictionary that puts the most
/// frequent tile at index 0 (ties keep source order for determinism).
fn build_dictionary_rank_remaps(holder: &FrameHolder) -> Result<Vec<Vec<u16>>> {
    let mut freq: Vec<Vec<u64>> = holder
        .dictionaries()
        .iter()
        .map(|d| vec![0u64; d.num_entries() as usize])
        .collect();
    for (idx, sprite) in holder.sprites().iter().enumerate() {
        if sprite.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(packed) = holder.packed_data(idx as u32) else {
            continue;
        };
        let f = freq
            .get_mut(sprite.dictionary_index as usize)
            .ok_or_else(|| {
                anyhow!(
                    "sprite {idx} references missing dictionary {}",
                    sprite.dictionary_index
                )
            })?;
        for &i in packed {
            let slot = f.get_mut(i as usize).ok_or_else(|| {
                anyhow!(
                    "sprite {idx} index {i} out of range for dictionary {}",
                    sprite.dictionary_index
                )
            })?;
            *slot += 1;
        }
    }
    Ok(freq
        .into_iter()
        .map(|f| {
            let mut order: Vec<u16> = (0..f.len() as u16).collect();
            order.sort_by_key(|&i| (std::cmp::Reverse(f[i as usize]), i));
            let mut remap = vec![0u16; f.len()];
            for (new, &old) in order.iter().enumerate() {
                remap[old as usize] = new as u16;
            }
            remap
        })
        .collect())
}

/// Apply an old→new index map to a dictionary's entries.
fn permute_dictionary(dict: &FrameDictionary, remap: &[u16]) -> FrameDictionary {
    let n = dict.num_entries();
    let mut values = vec![0u16; n as usize * 4];
    for old in 0..n {
        let new = remap[old as usize] as usize;
        values[new * 4..new * 4 + 4].copy_from_slice(dict.lookup_pixels(old));
    }
    FrameDictionary::from_raw(n, values)
}

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
        for sprite in &level_refs.animation_rhs {
            // Level patches, background FX, targets, and mobile children all
            // use FrameKind::Animation. Mirror resolve_rhs_path's ambiance
            // lookup and convert every authored variant that exists. The old
            // converter incorrectly looked under Characters/, which silently
            // omitted assets such as Animations/Day/chariot02.rhs.
            for rel in animation_rhs_paths(sprite) {
                if self.exists(&rel) {
                    self.convert_rel(&rel)?;
                }
            }
        }
        // Character banks are by far the largest conversion roots. Convert
        // them after the comparatively small level animation dependency set,
        // so interrupted/debug conversions still contain the assets needed
        // to inspect a level rather than tens of thousands of unrelated
        // character frames and no level FX.
        for name in &chars {
            if name.is_empty() {
                continue;
            }
            self.convert_rel(&format!("Characters/{name}.rhs"))?;
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
            sprite_script::SpriteScriptor::load_all_profiles(&src.to_string_lossy())
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
            "pixel_format": "legacy_color_keys",
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

fn animation_rhs_paths(sprite: &str) -> impl Iterator<Item = String> + '_ {
    [
        "Day", "Night", "Fog", "Attack", "Custom1", "Custom2", "Custom3", "Custom4", "",
    ]
    .into_iter()
    .map(move |subdir| {
        if subdir.is_empty() {
            format!("Animations/{sprite}.rhs")
        } else {
            format!("Animations/{subdir}/{sprite}.rhs")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AudioKind, add_character_action_rhs_profiles, animation_rhs_paths,
        animation_rhs_rel_existing, exclamation_dat_filename, insert_standalone_audio,
        level_asset_rel_existing, normalize_robin_profile_index, positional_pair_map,
        prepare_shipping_payload, transcode_audio_to_opus,
    };
    use robin_assets::shipping_datadir::ShippingMission;
    use robin_engine::profiles::{Action, CharacterProfile, ProfileManager};

    #[test]
    fn level_animation_rhs_paths_follow_runtime_ambiance_lookup() {
        let paths = animation_rhs_paths("chariot02").collect::<Vec<_>>();

        assert!(paths.contains(&"Animations/Day/chariot02.rhs".to_owned()));
        assert!(paths.contains(&"Animations/chariot02.rhs".to_owned()));
        assert!(paths.iter().all(|path| !path.starts_with("Characters/")));
    }

    #[test]
    fn shipping_animation_ambiance_uses_original_bit_values() {
        let exists = |path: &str| Some(path.into());

        assert_eq!(
            animation_rhs_rel_existing(1, "river", &exists),
            "Animations/Day/river.rhs"
        );
        assert_eq!(
            animation_rhs_rel_existing(2, "river", &exists),
            "Animations/Fog/river.rhs"
        );
        assert_eq!(
            animation_rhs_rel_existing(4, "river", &exists),
            "Animations/Night/river.rhs"
        );
        assert_eq!(
            animation_rhs_rel_existing(8, "river", &exists),
            "Animations/Day/river.rhs"
        );
    }

    #[test]
    fn shipping_level_assets_follow_exact_ambiance_day_root_lookup() {
        let existing = [
            "Levels/Attack/castle.map",
            "Levels/Day/castle.min",
            "Levels/root.map",
            "Levels/root.min",
        ];
        let exists = |path: &str| existing.contains(&path).then(|| path.into());

        assert_eq!(
            level_asset_rel_existing(8, "castle", ".map", &exists).unwrap(),
            "Levels/Attack/castle.map"
        );
        assert_eq!(
            level_asset_rel_existing(8, "castle", ".min", &exists).unwrap(),
            "Levels/Day/castle.min"
        );
        assert_eq!(
            level_asset_rel_existing(128, "root", ".map", &exists).unwrap(),
            "Levels/root.map"
        );
        assert!(level_asset_rel_existing(16, "missing", ".min", &exists).is_err());
    }

    #[test]
    fn robin_profile_normalization_uses_forest_flag() {
        let profiles = ProfileManager {
            characters: vec![
                CharacterProfile {
                    filename: "RobinHood".into(),
                    ..CharacterProfile::default()
                },
                CharacterProfile {
                    filename: "RobinTown".into(),
                    ..CharacterProfile::default()
                },
                CharacterProfile {
                    filename: "LittleJohn".into(),
                    ..CharacterProfile::default()
                },
            ],
            ..ProfileManager::new()
        };
        assert_eq!(
            normalize_robin_profile_index(&profiles, 1, true).unwrap(),
            0
        );
        assert_eq!(
            normalize_robin_profile_index(&profiles, 0, false).unwrap(),
            1
        );
        assert_eq!(
            normalize_robin_profile_index(&profiles, 2, true).unwrap(),
            2
        );
    }

    #[test]
    fn positional_pairing_drops_conflicting_variant_frames() {
        // Frame 10 pairs consistently with 20; frame 11 pairs with both 21
        // and 22 (a duplicated variant frame against different hub frames)
        // and must be dropped; frame 12 duplicates a consistent pair.
        let pairs = positional_pair_map(&[10, 11, 12, 11, 12], &[20, 21, 30, 22, 30]);
        assert_eq!(pairs.get(&10), Some(&20));
        assert_eq!(pairs.get(&11), None);
        assert_eq!(pairs.get(&12), Some(&30));
    }

    #[test]
    fn exclamation_id_maps_to_original_actor_table_name() {
        assert_eq!(
            exclamation_dat_filename(u32::from_le_bytes(*b"PCRH")),
            "actorPCRH.dat"
        );
    }

    #[test]
    fn character_actions_add_projectile_and_pickup_rhs_capabilities() {
        let mut required = std::collections::BTreeMap::new();
        add_character_action_rhs_profiles(
            &mut required,
            [Action::Bow, Action::Purse, Action::WaspNest],
        );
        for path in [
            "Characters/ACCESSORIES_Arrow.rhs",
            "Characters/BONUS_Arrows.rhs",
            "Characters/ACCESSORIES_MoneyBag.rhs",
            "Characters/ACCESSORIES_Coin.rhs",
            "Characters/BONUS_MoneyBag.rhs",
            "Characters/ACCESSORIES_Wasp.rhs",
            "Characters/ACCESSORIES_WaspSting.rhs",
            "Characters/BONUS_WaspsNest.rhs",
        ] {
            assert!(required.contains_key(path), "missing {path}");
        }
        assert!(!required.contains_key("Characters/RELIC_Crown.rhs"));
    }

    #[test]
    fn resume_reuses_only_an_exact_decoded_payload() {
        let temp = tempfile::tempdir().unwrap();
        let mut payload = ShippingMission::default();
        payload.raw.insert("one.bin".into(), vec![1, 2, 3]);
        let (filename, compressed) =
            prepare_shipping_payload(temp.path(), "Example", &payload, 30, false).unwrap();
        std::fs::write(temp.path().join(&filename), compressed.unwrap()).unwrap();

        let (reused_filename, compressed) =
            prepare_shipping_payload(temp.path(), "Example", &payload, 30, true).unwrap();
        assert_eq!(reused_filename, filename);
        assert!(compressed.is_none());

        let (_, compressed) =
            prepare_shipping_payload(temp.path(), "Example", &payload, 29, true).unwrap();
        assert!(
            compressed.is_some(),
            "a different zstd window must not reuse"
        );

        payload.raw.insert("two.bin".into(), vec![4]);
        let (_, compressed) =
            prepare_shipping_payload(temp.path(), "Example", &payload, 30, true).unwrap();
        assert!(compressed.is_some());
    }

    #[test]
    fn standalone_opus_is_cataloged_without_entering_mission_payload() {
        let temp = tempfile::tempdir().unwrap();
        let assets_dir = temp.path().join("audio/assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        let mut catalog = std::collections::BTreeMap::new();
        let payload = ShippingMission::default();
        let opus = b"OggS-fake-OpusHead-test-payload";

        insert_standalone_audio(
            &mut catalog,
            &assets_dir,
            "common",
            "Data/Sounds/Arrow.wav",
            opus,
            1_234,
        )
        .unwrap();

        assert!(payload.raw.is_empty());
        assert!(payload.audio_durations_ms.is_empty());
        let asset = catalog.get("sounds/arrow.opus").unwrap();
        assert_eq!(asset.encoded_size, opus.len() as u32);
        assert_eq!(asset.duration_ms, 1_234);
        assert_eq!(std::fs::read(temp.path().join(&asset.file)).unwrap(), opus);
        assert!(
            !robin_assets::shipping_datadir::encode_mission_native(&payload)
                .windows(opus.len())
                .any(|window| window == opus)
        );
    }

    #[test]
    #[ignore = "requires ffmpeg with libopus"]
    fn opus_transcode_is_byte_deterministic() {
        let sample_rate = 8_000u32;
        let sample_count = 800u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + sample_count * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(sample_count * 2).to_le_bytes());
        wav.resize(wav.len() + (sample_count * 2) as usize, 0);

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("determinism-fixture.wav");
        std::fs::write(&source, wav).unwrap();
        let first = transcode_audio_to_opus(&source, AudioKind::Voice).unwrap();
        let second = transcode_audio_to_opus(&source, AudioKind::Voice).unwrap();

        assert_eq!(first, second);
        assert!(first.starts_with(b"OggS"));
        assert!(first.windows(8).any(|window| window == b"OpusHead"));
        assert!(
            first
                .windows(b"robinhood-web-shipping".len())
                .any(|window| window == b"robinhood-web-shipping")
        );
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
    /// RHS basenames loaded through `FrameKind::Animation` from
    /// `Data/Animations[/<ambiance>]`, not `Data/Characters`.
    animation_rhs: BTreeSet<String>,
    map_names: BTreeSet<String>,
    /// Sound-source IDs referenced by each level's `.rhp`. The runtime
    /// maps each ID to a `snd_%03d.wav` file under `Data/Sounds/`.
    sound_wave_ids: BTreeSet<u32>,
}

fn collect_level_refs(proto: &LoadedProtoLevel, mission: &LoadedMission, out: &mut LevelRefs) {
    for p in &proto.patches {
        let n = &p.element_fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.animation_rhs.insert(n.clone());
        }
    }
    for fx in &proto.animations {
        let n = &fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.animation_rhs.insert(n.clone());
        }
    }
    if !mission.header.map_filename.is_empty() {
        out.map_names.insert(mission.header.map_filename.clone());
    }
    for p in &mission.mission_patches {
        let n = &p.element_fx.sprite.frame_profile_name;
        if !n.is_empty() {
            out.animation_rhs.insert(n.clone());
        }
    }
    for mobile in &mission.mobile_elements {
        for fx in &mobile.sprites {
            let n = &fx.sprite.frame_profile_name;
            if !n.is_empty() {
                out.animation_rhs.insert(n.clone());
            }
        }
    }
    // Targets are Data/Animations sprites resolved via `resolve_rhs_path`
    // (see `engine/level_loading.rs` :1255) — add their `filename` to the
    // referenced-sprite set so the converter emits the `.rhs` / `.bnk`
    // sources they need.
    for t in &mission.targets {
        if !t.filename.is_empty() {
            out.animation_rhs.insert(t.filename.clone());
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

use robin_assets::shipping_datadir::{
    RhsData, ShippingAudioAsset, ShippingDatadir, ShippingMission, ShippingMissionRef,
    ShippingSprite, ShippingSpriteBank, SpriteVqChunk,
};
use robin_engine::level_data::LoadedLevel;

#[derive(Default)]
struct ShippingMissionBuild {
    payload: ShippingMission,
    required_rhs_profiles: std::collections::BTreeMap<String, BTreeSet<String>>,
    required_exclamation_ids: BTreeSet<u32>,
    music_names: BTreeSet<String>,
    dialogue_samples: BTreeSet<String>,
    map_names: BTreeSet<String>,
    level_asset_keys: BTreeSet<String>,
    proto_filename: String,
    forest_level: bool,
    ambiance: u32,
}

/// One shared RHS chunk between requirement resolution and payload assembly.
struct RhsChunkPrep {
    /// Filtered profile metadata for the chunk, or `None` for a synthesized
    /// sprite-only family-base chunk no mission requires directly.
    rhs_data: Option<RhsData>,
    matched_profiles: usize,
    /// Frame ids of *all* profiles in RHS load order. Cross-variant pairing
    /// is positional over this order (a variant's frame tables mirror its
    /// family base's 1:1), so it includes unmatched profiles too.
    script_order: Vec<u32>,
    used_sprite_ids: BTreeSet<u32>,
    /// Family-base RHS rel when this chunk is coded cross-variant.
    base_rel: Option<String>,
    /// Variant bank id -> base bank id for the sprites coded against a base.
    base_ids: std::collections::BTreeMap<u32, u32>,
    /// Second-hub RHS rel when this chunk is star-2 coded (schema v10).
    base2_rel: Option<String>,
    /// Variant bank id -> second-predecessor bank id. Every key must also be
    /// present in `base_ids` (the codec requires base2 => base).
    base2_ids: std::collections::BTreeMap<u32, u32>,
}

/// Expected packed word count of a VQ sprite's `(width/4) x height` index
/// grid, or `None` when the dims cannot form one (zero-sized or ragged).
fn vq_grid_words(width: u16, height: u16) -> Option<usize> {
    (width > 0 && height > 0 && width.is_multiple_of(4))
        .then(|| (width as usize / 4) * height as usize)
}

/// A sprite's packed words after the dictionary rank permutation (identity
/// when ranking is disabled). `None` when the bank holds no data for it.
fn remapped_packed(
    holder: &FrameHolder,
    dict_remaps: Option<&[Vec<u16>]>,
    idx: u32,
) -> Result<Option<Vec<u16>>> {
    let sprite = holder
        .sprites()
        .get(idx as usize)
        .ok_or_else(|| anyhow!("sprite {idx} beyond the shipping bank"))?;
    let Some(packed) = holder.packed_data(idx) else {
        return Ok(None);
    };
    Ok(Some(match dict_remaps {
        Some(remaps) if sprite.dictionary_index != UNMAPPED_DICT => {
            let remap = remaps
                .get(sprite.dictionary_index as usize)
                .ok_or_else(|| {
                    anyhow!(
                        "sprite {idx} references dictionary {} without a rank remap",
                        sprite.dictionary_index
                    )
                })?;
            packed
                .iter()
                .map(|&i| {
                    remap.get(i as usize).copied().ok_or_else(|| {
                        anyhow!(
                            "sprite {idx} index {i} out of range for dictionary {}",
                            sprite.dictionary_index
                        )
                    })
                })
                .collect::<Result<Vec<u16>>>()?
        }
        _ => packed.to_vec(),
    }))
}

/// Assemble one shared RHS chunk: sprite rows for every reachable bank slot,
/// with all well-formed VQ grids coded into a single `sprite_codec` blob
/// (cross-variant against `prep.base_ids` where present) and RLE/ragged
/// sprites keeping raw packed words.
fn build_rhs_chunk_payload(
    holder: &FrameHolder,
    dict_remaps: Option<&[Vec<u16>]>,
    rel: &str,
    prep: &RhsChunkPrep,
) -> Result<ShippingMission> {
    let mut payload = ShippingMission::default();
    if let Some(rhs_data) = &prep.rhs_data {
        payload.rhs_files.insert(rel.to_owned(), rhs_data.clone());
    }
    let mut sprites = Vec::with_capacity(prep.used_sprite_ids.len());
    let mut blob_ids = Vec::new();
    let mut blob_dims = Vec::new();
    let mut blob_grids: Vec<Vec<u16>> = Vec::new();
    let mut blob_bases: Vec<Option<Vec<u16>>> = Vec::new();
    let mut blob_base_ids: Vec<Option<u32>> = Vec::new();
    let mut blob_base2s: Vec<Option<Vec<u16>>> = Vec::new();
    let mut blob_base2_ids: Vec<Option<u32>> = Vec::new();
    let mut alphabet: u16 = 0;
    for &idx in &prep.used_sprite_ids {
        let sprite = holder
            .sprites()
            .get(idx as usize)
            .ok_or_else(|| anyhow!("RHS {rel} references sprite {idx} beyond the shipping bank"))?;
        let row_packed = match remapped_packed(holder, dict_remaps, idx)? {
            Some(packed)
                if sprite.dictionary_index != UNMAPPED_DICT
                    && vq_grid_words(sprite.width, sprite.height) == Some(packed.len()) =>
            {
                let dict = holder.dictionary(sprite.dictionary_index).ok_or_else(|| {
                    anyhow!(
                        "sprite {idx} references missing dictionary {}",
                        sprite.dictionary_index
                    )
                })?;
                alphabet = alphabet.max(dict.num_entries());
                match prep.base_ids.get(&idx) {
                    Some(&base_id) => {
                        let base =
                            remapped_packed(holder, dict_remaps, base_id)?.ok_or_else(|| {
                                anyhow!(
                                    "family base sprite {base_id} for RHS {rel} has no packed data"
                                )
                            })?;
                        blob_bases.push(Some(base));
                        blob_base_ids.push(Some(base_id));
                    }
                    None => {
                        blob_bases.push(None);
                        blob_base_ids.push(None);
                    }
                }
                match prep.base2_ids.get(&idx) {
                    Some(&base2_id) => {
                        if !prep.base_ids.contains_key(&idx) {
                            bail!(
                                "sprite {idx} of RHS {rel} plans a base2 predecessor without a base"
                            );
                        }
                        let base2 =
                            remapped_packed(holder, dict_remaps, base2_id)?.ok_or_else(|| {
                                anyhow!(
                                    "family base2 sprite {base2_id} for RHS {rel} has no packed \
                                     data"
                                )
                            })?;
                        blob_base2s.push(Some(base2));
                        blob_base2_ids.push(Some(base2_id));
                    }
                    None => {
                        blob_base2s.push(None);
                        blob_base2_ids.push(None);
                    }
                }
                blob_ids.push(idx);
                blob_dims.push((sprite.width / 4, sprite.height));
                blob_grids.push(packed);
                Vec::new()
            }
            Some(packed) => {
                if sprite.dictionary_index != UNMAPPED_DICT {
                    // VQ words that disagree with the sprite's grid shape
                    // cannot ride the codec blob; ship them raw rather than
                    // guessing at dimensions.
                    tracing::warn!(
                        rhs = rel,
                        sprite = idx,
                        words = packed.len(),
                        width = sprite.width,
                        height = sprite.height,
                        "VQ sprite length does not match its grid; keeping raw indices"
                    );
                }
                packed
            }
            None if sprite.width == 0 || sprite.height == 0 => Vec::new(),
            None => bail!("RHS {rel} references non-empty sprite {idx} with no packed bank data"),
        };
        sprites.push((
            idx,
            ShippingSprite {
                width: sprite.width,
                height: sprite.height,
                dictionary_index: sprite.dictionary_index,
                packed_data: Arc::new(row_packed),
            },
        ));
    }
    let coded_sprites = blob_ids.len();
    let mut vq_chunks = Vec::new();
    let mut blob_bytes = 0usize;
    if !blob_ids.is_empty() {
        let grids: Vec<robin_assets::sprite_codec::SpriteGrid> = blob_dims
            .iter()
            .zip(&blob_grids)
            .map(
                |(&(cols, rows), grid)| robin_assets::sprite_codec::SpriteGrid {
                    cols,
                    rows,
                    indices: grid,
                },
            )
            .collect();
        let bases: Vec<Option<&[u16]>> = blob_bases.iter().map(|base| base.as_deref()).collect();
        let base2s: Vec<Option<&[u16]>> = blob_base2s.iter().map(|base| base.as_deref()).collect();
        let has_base2 = blob_base2_ids.iter().any(Option::is_some);
        // Standalone chunks gain within-chunk self-references (temporal /
        // adjacent-direction), derived from the SHIPPED profile set — the
        // decoder re-derives the identical map from the chunk's RhsData, so
        // the rule must run over exactly what ships.
        let selfrefs: Vec<Option<robin_assets::sprite_codec::SelfRef>> =
            match (&prep.rhs_data, &prep.base_rel) {
                (Some(rhs_data), None) => robin_assets::shipping_datadir::derive_chunk_self_refs(
                    &rhs_data.profiles,
                    &blob_ids,
                ),
                _ => vec![None; blob_ids.len()],
            };
        let has_self_refs = selfrefs.iter().any(Option::is_some);
        let blob = robin_assets::sprite_codec::encode_grids_shipping(
            alphabet,
            &grids,
            Some(&bases),
            Some(&base2s),
            &selfrefs,
        )
        .with_context(|| format!("encode VQ sprite grids for {rel}"))?;
        blob_bytes = blob.len();
        if has_base2 && prep.base2_rel.is_none() {
            bail!("RHS {rel} coded base2 sprites without a planned base2 chunk");
        }
        vq_chunks.push(SpriteVqChunk {
            rhs: rel.to_owned(),
            base_rhs: prep.base_rel.clone(),
            base2_rhs: if has_base2 {
                prep.base2_rel.clone().unwrap_or_default()
            } else {
                String::new()
            },
            alphabet,
            sprite_ids: blob_ids,
            base_ids: blob_base_ids,
            base2_ids: if has_base2 {
                blob_base2_ids
            } else {
                Vec::new()
            },
            self_refs: has_self_refs,
            blob,
        });
    }
    payload.sprite_bank = Some(ShippingSpriteBank {
        signature: holder.signature(),
        dictionaries: Vec::new(),
        sprite_count: holder.sprites().len() as u32,
        sprites,
        vq_chunks,
    });
    tracing::info!(
        rhs = rel,
        sprites = prep.used_sprite_ids.len(),
        required_rhs_profiles = prep.matched_profiles,
        vq_sprites = coded_sprites,
        vq_blob_bytes = blob_bytes,
        base = prep.base_rel.as_deref().unwrap_or(""),
        base2 = prep.base2_rel.as_deref().unwrap_or(""),
        "built shared RHS sprite payload"
    );
    Ok(payload)
}

fn convert_shipping(data_in: PathBuf, data_out: &Path, opts: ShippingOpts) -> Result<()> {
    let mut dd = ShippingDatadir::default();
    let mut beggar_ids: BTreeSet<u32> = BTreeSet::new();
    let audio_assets_dir = data_out.join("audio/assets");
    fs::create_dir_all(&audio_assets_dir)?;

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
            mgr.disable_recovery_for_shipping();
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
    // Menu sounds are part of the data artifact, not the wasm executable.
    // Keep them in the boot manifest because they are needed before any
    // mission dependency is selected.
    let mut boot_audio = ShippingMission::default();
    let mut menu_roots = vec![data_in.join("Sounds/Menu")];
    menu_roots.extend(
        locale_dirs
            .iter()
            .map(|locale| locale.data_dir.join("Sounds/Menu")),
    );
    for root in menu_roots {
        if !root.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_files_recursive(&root, &mut files)?;
        files.sort();
        for path in files {
            let filename = path
                .strip_prefix(&root)
                .expect("menu audio must remain below its collection root");
            let relative = Path::new("Sounds/Menu").join(filename);
            insert_shipping_audio(
                &mut boot_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "menu",
                &relative.to_string_lossy(),
                &path,
                AudioKind::Effect,
                opts.audio_format,
            )?;
        }
    }
    if let Some((relative, path)) = ["wav", "ogg"].into_iter().find_map(|extension| {
        let relative = format!("Musics/Menu.{extension}");
        in_path(&relative).map(|path| (relative, path))
    }) {
        insert_shipping_audio(
            &mut boot_audio,
            &mut dd.audio_assets,
            &audio_assets_dir,
            "menu",
            &relative,
            &path,
            AudioKind::Music,
            opts.audio_format,
        )?;
    } else {
        bail!("required menu music Musics/Menu.{{wav,ogg}} is missing");
    }
    dd.raw.extend(boot_audio.raw);
    dd.audio_durations_ms.extend(boot_audio.audio_durations_ms);

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
    let character_exclamation_ids: Vec<u32> = cpf
        .characters
        .iter()
        .map(|profile| profile.exclamation_id)
        .collect();
    for (i, c) in cpf.civilians.iter().enumerate() {
        if c.civilian_type == CivilianType::Beggar {
            beggar_ids.insert(i as u32);
        }
    }

    // Missions → .rhp/.rhm/.scb/.red, also follow level sprite refs.
    let mut mission_builds = std::collections::BTreeMap::<String, ShippingMissionBuild>::new();

    // Mission descriptors are authoritative campaign/UI data even when the
    // profile deliberately points at the non-loadable `Impossible_mission`
    // sentinel. Parse them independently from the RHP/RHM payload loop.
    for mp in &cpf.missions {
        let filename = res_descr::red_filename(mp.id);
        let red_rel = format!("Text/{filename}");
        if let Some(red_path) = in_path(&red_rel) {
            dd.red_files
                .insert(filename, res_descr::load(&red_path.to_string_lossy())?);
        } else {
            // Some stock profiles have no descriptor in the source install.
            // Preserve that absence; never synthesize authoritative UI data.
            tracing::warn!(
                profile_id = mp.id,
                "source mission descriptor is absent: {red_rel}"
            );
        }
    }

    // Dialogue descriptors refer to WAVE tables in the localized Level.res.
    // Resolve those tables while building each mission's dependency closure;
    // shipping every locale's dialogue at boot would defeat split loading.
    let level_res_path =
        in_path("Text/Level.res").ok_or_else(|| anyhow!("Text/Level.res missing"))?;
    let mut level_res = ResourceManager::new();
    level_res.attach_resource_file(&level_res_path.to_string_lossy())?;

    for mp in &cpf.missions {
        if mp.proto_level_filename.is_empty() || mp.mission_filename.is_empty() {
            continue;
        }
        let rhp_rel = format!("Levels/{}.rhp", mp.proto_level_filename);
        let rhm_rel = format!("Levels/{}.rhm", mp.mission_filename);
        let scb_rel = format!("Levels/{}.scb", mp.mission_filename);

        let Some(rhp_path) = in_path(&rhp_rel) else {
            tracing::warn!("missing: {}", rhp_rel);
            continue;
        };
        let Some(rhm_path) = in_path(&rhm_rel) else {
            tracing::warn!("missing: {}", rhm_rel);
            continue;
        };

        let (proto, mission) = parse_level_pair(&rhp_path, &rhm_path, &beggar_ids)?;
        let forest_level = proto
            .misc
            .as_ref()
            .ok_or_else(|| {
                anyhow!(
                    "proto level {} has no MISC forest-level metadata",
                    mp.proto_level_filename
                )
            })?
            .forest_level;
        let mut build = ShippingMissionBuild {
            proto_filename: mp.proto_level_filename.clone(),
            forest_level,
            ambiance: mission.header.ambiance,
            ..ShippingMissionBuild::default()
        };
        build.music_names.extend(
            [&mp.green_music, &mp.yellow_music, &mp.red_music]
                .into_iter()
                .filter(|name| !name.is_empty())
                .cloned(),
        );
        let red_filename = res_descr::red_filename(mp.id);
        if let Some(descriptors) = dd.red_files.get(&red_filename) {
            for (dialogue_index, dialogue) in descriptors.dialogues.iter().enumerate() {
                for sentence_index in 0..dialogue.portrait_ids.len() {
                    match level_res.get_sample(dialogue.sound_table_id, sentence_index) {
                        Ok(sample) if !sample.is_empty() => {
                            build
                                .dialogue_samples
                                .insert(format!("Text/{}", sample.replace('\\', "/")));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!(
                                    "resolve dialogue sample for mission {} dialogue {dialogue_index} sentence {sentence_index}",
                                    mp.mission_filename
                                )
                            });
                        }
                    }
                }
            }
        }
        let required_rhs_profiles = &mut build.required_rhs_profiles;
        // Demo boot hardcodes its party; preserve those profiles even when
        // the mission script does not name them directly.
        if mp.mission_filename == "Dem_Lei_MP" {
            add_required_pc_profiles_for_pcs(
                required_rhs_profiles,
                &cpf,
                "RJMT",
                forest_level,
                &in_path,
            );
        } else if mp.mission_filename == "Demo_Lin" {
            add_required_pc_profiles_for_pcs(
                required_rhs_profiles,
                &cpf,
                "RSABC",
                forest_level,
                &in_path,
            );
        }
        // Collect sprite/map refs.
        for p in &proto.patches {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for fx in &proto.animations {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &fx.sprite,
                &in_path,
            );
        }
        if !mission.header.map_filename.is_empty() {
            build.map_names.insert(mission.header.map_filename.clone());
        }
        for &idx in &mp.required_character_indices {
            let idx = normalize_robin_profile_index(&cpf, idx as usize, forest_level)?;
            add_required_character_rhs_profiles_for_index(
                required_rhs_profiles,
                &cpf,
                idx,
                &in_path,
            );
            if let Some(profile) = cpf.characters.get(idx)
                && profile.exclamation_id != 0
            {
                build
                    .required_exclamation_ids
                    .insert(profile.exclamation_id);
            }
        }
        for p in &mission.mission_patches {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for target in &mission.targets {
            let rel =
                animation_rhs_rel_existing(mission.header.ambiance, &target.filename, &in_path);
            if in_path(&rel).is_some() {
                add_required_rhs_rel(required_rhs_profiles, rel, &target.profile_name);
            } else {
                // TODO: Determine why a few stock level records name an RHS
                // that does not exist in any original animation directory.
                tracing::warn!(
                    mission = mp.mission_filename,
                    "source target RHS is absent: {rel}"
                );
            }
        }
        for mobile in &mission.mobile_elements {
            for fx in &mobile.sprites {
                add_required_animation_rhs_profile(
                    required_rhs_profiles,
                    mission.header.ambiance,
                    &fx.sprite,
                    &in_path,
                );
            }
        }
        for soldier in &mission.soldiers {
            if let Some(profile) = cpf.soldiers.get(soldier.profile_number as usize) {
                if profile.exclamation_id != 0 {
                    build
                        .required_exclamation_ids
                        .insert(profile.exclamation_id);
                }
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for civilian in &mission.civilians {
            if let Some(profile) = cpf.civilians.get(civilian.profile_number as usize) {
                if profile.exclamation_id != 0 {
                    build
                        .required_exclamation_ids
                        .insert(profile.exclamation_id);
                }
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for pc in &mission.pcs_to_rescue {
            let profile_index =
                normalize_robin_profile_index(&cpf, pc.profile_index as usize, forest_level)?;
            add_required_character_rhs_profiles_for_index(
                required_rhs_profiles,
                &cpf,
                profile_index,
                &in_path,
            );
            if let Some(profile) = cpf.characters.get(profile_index)
                && profile.exclamation_id != 0
            {
                build
                    .required_exclamation_ids
                    .insert(profile.exclamation_id);
            }
        }
        for bonus in &mission.bonuses {
            if let Some((file, profile)) = bonus_type_to_sprite_asset_for_shipping(bonus.bonus_type)
            {
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{file}.rhs"),
                    profile,
                );
            }
        }
        if !mission.scrolls.is_empty() {
            add_required_rhs_rel(
                required_rhs_profiles,
                "Characters/BONUS_Parchment.rhs",
                "BONUS Parchemin",
            );
            add_required_rhs_rel(
                required_rhs_profiles,
                "Characters/BONUS_FourLeavedClover.rhs",
                "BONUS Trefle",
            );
        }
        add_required_rhs_rel(required_rhs_profiles, "Characters/Blip00.rhs", "Blip 00");
        // The original engine creates every object master at level load.
        // These payloads are tiny and must be present now that parsed RHS is
        // authoritative and no raw-file fallback exists.
        add_all_saved_world_object_rhs_profiles(required_rhs_profiles);

        build
            .payload
            .levels
            .insert(mp.mission_filename.clone(), LoadedLevel { proto, mission });

        if let Some(p) = in_path(&scb_rel) {
            let parsed = scb::parse_file(&p).map_err(|e| anyhow!("scb: {e}"))?;
            build
                .payload
                .scripts
                .insert(mp.mission_filename.clone(), parsed);
        } else {
            tracing::warn!("missing: {}", scb_rel);
        }
        mission_builds.insert(mp.mission_filename.clone(), build);
    }

    // Runtime party composition is not known during conversion. Build a
    // manifest index for every character profile so the mission boundary can
    // fetch only the selected team plus eligible reinforcement candidates.
    // Each entry also carries the projectile/pickup masters enabled by that
    // profile's actions; those objects can be created during a tick and cannot
    // perform asynchronous loading themselves.
    let mut character_rhs_requirements = std::collections::BTreeMap::<
        u32,
        std::collections::BTreeMap<String, BTreeSet<String>>,
    >::new();
    for (index, profile) in cpf.characters.iter().enumerate() {
        let profile_index = u32::try_from(index).context("character profile index exceeds u32")?;
        let required = character_rhs_requirements.entry(profile_index).or_default();
        add_character_rhs_profiles_for_index(required, &cpf, index, &in_path, false);
        add_character_action_rhs_profiles(
            required,
            profile
                .actions
                .into_iter()
                .chain(profile.contextual_actions),
        );
    }

    // A decoded save can contain a live object which is neither authored by
    // the destination mission nor implied by its current party. Until exact
    // saved-world object types are threaded into this boundary, keep the full
    // object-master closure explicit and load it only for save launches.
    let mut saved_world_rhs_requirements =
        std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    add_all_saved_world_object_rhs_profiles(&mut saved_world_rhs_requirements);

    // Load the source bank once. Each RHS gets one shared payload containing
    // its metadata and reachable bank slots; missions reference these files
    // instead of duplicating characters they have in common.
    let parent = data_in
        .parent()
        .ok_or_else(|| anyhow!("data dir has no parent"))?;
    let holder =
        FrameHolder::from_data_dir(&parent.to_string_lossy()).context("loading sprite bank")?;
    // Frequency-rank the dictionaries so the most used tile of each becomes
    // index 0, and remember the old→new maps to rewrite every VQ sprite's
    // indices below. A consistent permutation is invisible to the decoder.
    let dict_remaps = if opts.rank_dictionaries {
        Some(build_dictionary_rank_remaps(&holder)?)
    } else {
        None
    };
    let shipping_dictionaries = match &dict_remaps {
        Some(remaps) => holder
            .dictionaries()
            .iter()
            .zip(remaps)
            .map(|(dict, remap)| permute_dictionary(dict, remap))
            .collect(),
        None => holder.dictionaries().to_vec(),
    };
    dd.sprite_bank = Some(ShippingSpriteBank {
        signature: holder.signature(),
        dictionaries: shipping_dictionaries,
        sprite_count: holder.sprites().len() as u32,
        sprites: Vec::new(),
        vq_chunks: Vec::new(),
    });
    let mut rhs_requirements = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for build in mission_builds.values() {
        for (rel, profiles) in &build.required_rhs_profiles {
            rhs_requirements
                .entry(rel.clone())
                .or_default()
                .extend(profiles.iter().cloned());
        }
    }
    for required in character_rhs_requirements.values() {
        for (rel, profiles) in required {
            rhs_requirements
                .entry(rel.clone())
                .or_default()
                .extend(profiles.iter().cloned());
        }
    }
    for (rel, profiles) in &saved_world_rhs_requirements {
        rhs_requirements
            .entry(rel.clone())
            .or_default()
            .extend(profiles.iter().cloned());
    }
    // Max-level zstd and the VQ context-model encoder are deliberately
    // expensive and memory hungry. Bound the worker count; each completed
    // chunk is written in its worker so the result vectors retain only small
    // manifest metadata, not every compressed RHS.
    let compression_workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(4);
    let compression_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(compression_workers)
        .thread_name(|index| format!("shipping-zstd-{index}"))
        .build()
        .context("create bounded shipping compression pool")?;

    // Phase A: load each required RHS and resolve which bank sprites its
    // matched profiles reach. Payload assembly happens after the family pass
    // below so variant chunks can be coded against their family base.
    let mut rhs_preps = std::collections::BTreeMap::<String, RhsChunkPrep>::new();
    for (rel, required_profiles) in &rhs_requirements {
        if rel.is_empty() {
            continue;
        }
        let path = in_path(rel)
            .ok_or_else(|| anyhow!("required authoritative shipping RHS is missing: {rel}"))?;
        let mut used_sprite_ids = BTreeSet::<u32>::new();
        let (signature, profiles) =
            sprite_script::SpriteScriptor::load_all_profiles(&path.to_string_lossy())
                .map_err(|error| anyhow!("rhs {rel}: {error}"))?;
        let mut script_order = Vec::new();
        for (_, info) in &profiles {
            for script in info.scripts.iter() {
                script_order.extend_from_slice(&script.frame_ids);
            }
        }
        let all_profiles_required = required_profiles.contains("");
        let mut matched_profiles = BTreeSet::new();
        for (profile_name, info) in &profiles {
            if all_profiles_required || required_profiles.contains(profile_name) {
                matched_profiles.insert(profile_name.clone());
                for script in info.scripts.iter() {
                    used_sprite_ids.extend(script.frame_ids.iter().copied());
                }
            }
        }
        for required in required_profiles {
            if !required.is_empty() && !matched_profiles.contains(required) {
                bail!("authoritative shipping RHS {rel} is missing required profile '{required}'");
            }
        }
        let profiles = profiles
            .into_iter()
            .filter(|(name, _)| all_profiles_required || matched_profiles.contains(name))
            .collect();
        rhs_preps.insert(
            rel.clone(),
            RhsChunkPrep {
                rhs_data: Some(RhsData {
                    signature,
                    profiles,
                }),
                matched_profiles: matched_profiles.len(),
                script_order,
                used_sprite_ids,
                base_rel: None,
                base_ids: std::collections::BTreeMap::new(),
                base2_rel: None,
                base2_ids: std::collections::BTreeMap::new(),
            },
        );
    }

    // Phase B: variant families among Characters/*.rhs (the probe's corpus
    // rule: trailing-two-digit stem shared by more than one file; base = the
    // lexicographically first member). Variant chunks are coded against the
    // base's positionally aligned grids — measured 3.9x smaller than zstd on
    // that half of the character corpus (docs/COMPRESSION.md, 2026-08-28).
    let mut disk_character_names: Vec<String> = Vec::new();
    if let Some(dir) = resolve_case_insensitive(&data_in.join("Characters")).filter(|p| p.is_dir())
    {
        for entry in fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rhs"))
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                disk_character_names.push(stem.to_owned());
            }
        }
    }
    disk_character_names.sort();
    let family_key = |name: &str| -> Option<String> {
        let stripped = name.trim_end_matches(|c: char| c.is_ascii_digit());
        (stripped.len() + 2 == name.len() && !stripped.is_empty()).then(|| stripped.to_owned())
    };
    let mut families = std::collections::BTreeMap::<String, Vec<String>>::new();
    for name in &disk_character_names {
        if let Some(key) = family_key(name) {
            families.entry(key).or_default().push(name.clone());
        }
    }
    families.retain(|_, members| members.len() > 1);

    // Measured base selection (docs/COMPRESSION.md 2026-08-29): the
    // lexicographically-first member is the best coding hub in only 1 of 9
    // fullgame families; choosing the base that minimizes a sampled
    // conditional-entropy proxy — H(base | above) for the base itself plus
    // H(member | base tile) for every other member — recovers ~4% of the
    // family half of the corpus at zero format cost (the chunk already
    // records `base_rhs`). Falls back to the first member when the proxy
    // cannot be computed (e.g. indices beyond the 12-bit proxy key).
    let mut member_orders = std::collections::BTreeMap::<String, Vec<u32>>::new();
    for members in families.values() {
        for name in members {
            if member_orders.contains_key(name) {
                continue;
            }
            let rel = format!("Characters/{name}.rhs");
            let path = in_path(&rel)
                .ok_or_else(|| anyhow!("family member RHS {rel} missing from the datadir"))?;
            let (_, profiles) =
                sprite_script::SpriteScriptor::load_all_profiles(&path.to_string_lossy())
                    .map_err(|error| anyhow!("rhs {rel}: {error}"))?;
            let mut order = Vec::new();
            for (_, info) in &profiles {
                for script in info.scripts.iter() {
                    order.extend_from_slice(&script.frame_ids);
                }
            }
            member_orders.insert(name.clone(), order);
        }
    }
    // First-load weighting: a hub chunk that missions already require adds
    // nothing to their closures, while a dependency-only hub adds its whole
    // (large, standalone-coded) chunk — measured ~7 MB extra on H01's first
    // load with pure compression-optimal hubs. Among candidates within
    // FAMILY_HUB_PROXY_TOLERANCE of the best compression proxy, prefer the
    // member the most missions reference.
    const FAMILY_HUB_PROXY_TOLERANCE: f64 = 1.05;
    let mission_use_count = |name: &str| -> usize {
        let rel = format!("Characters/{name}.rhs");
        mission_builds
            .values()
            .filter(|build| {
                build
                    .required_rhs_profiles
                    .keys()
                    .any(|required| required.eq_ignore_ascii_case(&rel))
            })
            .count()
    };
    let pick_weighted = |costs: &[(f64, &String)]| -> Option<String> {
        let best = costs
            .iter()
            .map(|(cost, _)| *cost)
            .fold(f64::INFINITY, f64::min);
        costs
            .iter()
            .filter(|(cost, _)| *cost <= best * FAMILY_HUB_PROXY_TOLERANCE)
            .max_by(|(ca, na), (cb, nb)| {
                mission_use_count(na)
                    .cmp(&mission_use_count(nb))
                    .then(cb.total_cmp(ca))
                    .then_with(|| nb.cmp(na))
            })
            .map(|(_, name)| (*name).clone())
    };
    let mut family_bases = std::collections::BTreeMap::<String, String>::new();
    for (key, members) in &families {
        let mut costs: Vec<(f64, &String)> = Vec::new();
        let mut proxy_failed = false;
        for candidate in members {
            let mut cost = match family_base_standalone_proxy(&holder, &member_orders[candidate]) {
                Some(bits) => bits,
                None => {
                    proxy_failed = true;
                    break;
                }
            };
            for member in members {
                if member == candidate {
                    continue;
                }
                match family_base_pair_proxy(
                    &holder,
                    &member_orders[candidate],
                    &member_orders[member],
                ) {
                    Some(bits) => cost += bits,
                    None => {
                        proxy_failed = true;
                        break;
                    }
                }
            }
            if proxy_failed {
                break;
            }
            costs.push((cost, candidate));
        }
        let base = match (proxy_failed, pick_weighted(&costs)) {
            (false, Some(name)) => name,
            _ => {
                tracing::warn!(
                    family = key.as_str(),
                    "family base proxy unavailable; falling back to first member"
                );
                members[0].clone()
            }
        };
        tracing::info!(
            family = key.as_str(),
            base = base.as_str(),
            missions_using = mission_use_count(&base),
            "selected family coding base"
        );
        family_bases.insert(key.clone(), base);
    }

    // Star-2 topology (schema v10, docs/COMPRESSION.md 2026-08-29): family
    // members after the first two code each tile against TWO already-decoded
    // siblings — measured -22..25% on third-and-later members. hub1 is the
    // proxy-selected base above; hub2 is the member (excluding hub1) that is
    // the best SECOND predictor for the remaining members: argmin over
    // candidates c != hub1 of sum over members m not in {hub1, c} of
    // H(m | c tile). hub2's own chunk keeps coding against hub1 only.
    // Two-member families have no "third-and-later" members and skip this.
    let mut family_second_bases = std::collections::BTreeMap::<String, String>::new();
    for (key, members) in &families {
        if members.len() < 3 {
            continue;
        }
        let hub1 = &family_bases[key];
        let mut costs: Vec<(f64, &String)> = Vec::new();
        let mut proxy_failed = false;
        for candidate in members {
            if candidate == hub1 {
                continue;
            }
            let mut cost = 0.0;
            for member in members {
                if member == candidate || member == hub1 {
                    continue;
                }
                match family_base_pair_proxy(
                    &holder,
                    &member_orders[candidate],
                    &member_orders[member],
                ) {
                    Some(bits) => cost += bits,
                    None => {
                        proxy_failed = true;
                        break;
                    }
                }
            }
            if proxy_failed {
                break;
            }
            costs.push((cost, candidate));
        }
        match (proxy_failed, pick_weighted(&costs)) {
            (false, Some(name)) => {
                tracing::info!(
                    family = key.as_str(),
                    base2 = name.as_str(),
                    missions_using = mission_use_count(&name),
                    "selected family second base"
                );
                family_second_bases.insert(key.clone(), name);
            }
            _ => {
                tracing::warn!(
                    family = key.as_str(),
                    "family second-base proxy unavailable; coding this family star-1"
                );
            }
        }
    }

    // Lowercased variant name -> disk-cased base name. CPF-derived rels and
    // on-disk filenames can disagree in case, so matching is case-blind.
    let variant_base_names: std::collections::BTreeMap<String, String> = families
        .iter()
        .flat_map(|(key, members)| {
            let base = family_bases[key].clone();
            members
                .iter()
                .filter({
                    let base = base.clone();
                    move |name| **name != base
                })
                .map(move |name| (name.to_ascii_lowercase(), base.clone()))
        })
        .collect();
    // Lowercased third-and-later member name -> disk-cased second-hub name.
    // hub2 itself keeps coding against hub1 only, so it is excluded here.
    let variant_base2_names: std::collections::BTreeMap<String, String> = families
        .iter()
        .filter_map(|(key, members)| {
            let hub2 = family_second_bases.get(key)?;
            let hub1 = &family_bases[key];
            Some(
                members
                    .iter()
                    .filter(move |name| *name != hub1 && *name != hub2)
                    .map(move |name| (name.to_ascii_lowercase(), hub2.clone())),
            )
        })
        .flatten()
        .collect();

    let prep_rels: Vec<String> = rhs_preps.keys().cloned().collect();
    let mut loaded_base_script_orders = std::collections::BTreeMap::<String, Vec<u32>>::new();
    struct PlannedVariant {
        rel: String,
        base_rel: String,
        base_ids: std::collections::BTreeMap<u32, u32>,
        base2_rel: Option<String>,
        base2_ids: std::collections::BTreeMap<u32, u32>,
        /// Hub chunk rels this variant's decode depends on at install time.
        dep_rels: Vec<String>,
    }
    let mut planned_variants = Vec::<PlannedVariant>::new();
    let mut base_extra_ids = std::collections::BTreeMap::<String, BTreeSet<u32>>::new();
    for rel in &prep_rels {
        let Some(name) = rel
            .strip_prefix("Characters/")
            .and_then(|n| n.strip_suffix(".rhs"))
        else {
            continue;
        };
        let Some(base_name) = variant_base_names.get(&name.to_ascii_lowercase()) else {
            continue;
        };
        let base_rel = resolve_family_hub_rel(
            &prep_rels,
            &rhs_preps,
            &mut loaded_base_script_orders,
            &in_path,
            base_name,
            rel,
        )?;
        // Second hub, when this member is third-or-later in a star-2 family.
        let base2_rel = match variant_base2_names.get(&name.to_ascii_lowercase()) {
            Some(hub2_name) => Some(resolve_family_hub_rel(
                &prep_rels,
                &rhs_preps,
                &mut loaded_base_script_orders,
                &in_path,
                hub2_name,
                rel,
            )?),
            None => None,
        };
        let hub_script = |hub_rel: &str| {
            rhs_preps
                .get(hub_rel)
                .map(|prep| prep.script_order.as_slice())
                .or_else(|| loaded_base_script_orders.get(hub_rel).map(Vec::as_slice))
                .expect("hub script order resolved above")
        };
        let variant_prep = rhs_preps.get(rel).expect("prep listed in prep_rels");
        // Positional pairing over the script frame-id tables (the variant's
        // tables mirror each hub's 1:1); duplicated variant frames that pair
        // with conflicting hub frames fall back per hub.
        let pair_base = positional_pair_map(&variant_prep.script_order, hub_script(&base_rel));
        let pair_base2 = base2_rel
            .as_deref()
            .map(|hub2_rel| positional_pair_map(&variant_prep.script_order, hub_script(hub2_rel)));
        let mut base_ids = std::collections::BTreeMap::<u32, u32>::new();
        let mut base2_ids = std::collections::BTreeMap::<u32, u32>::new();
        let mut hub1_used = BTreeSet::<u32>::new();
        let mut hub2_used = BTreeSet::<u32>::new();
        let (mut vq_total, mut unbased) = (0usize, 0usize);
        for &vid in &variant_prep.used_sprite_ids {
            let sprite = holder.sprites().get(vid as usize).ok_or_else(|| {
                anyhow!("RHS {rel} references sprite {vid} beyond the shipping bank")
            })?;
            let Some(packed) = holder.packed_data(vid) else {
                continue;
            };
            if sprite.dictionary_index == UNMAPPED_DICT
                || vq_grid_words(sprite.width, sprite.height) != Some(packed.len())
            {
                continue;
            }
            vq_total += 1;
            let aligned = |bid: &u32| {
                let Some(base_sprite) = holder.sprites().get(*bid as usize) else {
                    return false;
                };
                let Some(base_packed) = holder.packed_data(*bid) else {
                    return false;
                };
                base_sprite.dictionary_index != UNMAPPED_DICT
                    && (base_sprite.width, base_sprite.height) == (sprite.width, sprite.height)
                    && base_packed.len() == packed.len()
            };
            let b1 = pair_base.get(&vid).copied().filter(aligned);
            let b2 = pair_base2
                .as_ref()
                .and_then(|pairs| pairs.get(&vid))
                .copied()
                .filter(aligned);
            // The probe's `code3` ladder: both aligned predecessors when
            // possible; a sprite aligning with only one hub takes that hub as
            // its single base (base ids are plain bank ids, so a hub2 sprite
            // works as a primary base); otherwise standalone.
            match (b1, b2) {
                (Some(b1), Some(b2)) => {
                    base_ids.insert(vid, b1);
                    base2_ids.insert(vid, b2);
                    hub1_used.insert(b1);
                    hub2_used.insert(b2);
                }
                (Some(b1), None) => {
                    base_ids.insert(vid, b1);
                    hub1_used.insert(b1);
                }
                (None, Some(b2)) => {
                    base_ids.insert(vid, b2);
                    hub2_used.insert(b2);
                }
                (None, None) => unbased += 1,
            }
        }
        if base_ids.is_empty() || unbased * 10 > vq_total {
            tracing::info!(
                rhs = rel.as_str(),
                base = base_rel.as_str(),
                base2 = base2_rel.as_deref().unwrap_or(""),
                vq = vq_total,
                unbased,
                "family variant pairs poorly with its hubs; coding standalone"
            );
            continue;
        }
        base_extra_ids
            .entry(base_rel.clone())
            .or_default()
            .extend(hub1_used.iter().copied());
        if let Some(hub2_rel) = base2_rel.as_ref().filter(|_| !hub2_used.is_empty()) {
            base_extra_ids
                .entry(hub2_rel.clone())
                .or_default()
                .extend(hub2_used.iter().copied());
        }
        // Dependency edges: hub1 always (hub2's own chunk decodes against
        // hub1, so hub1 must be in the closure whenever hub2 is), plus hub2
        // when any of its grids are referenced.
        let mut dep_rels = vec![base_rel.clone()];
        if let Some(hub2_rel) = base2_rel.as_ref().filter(|_| !hub2_used.is_empty()) {
            dep_rels.push(hub2_rel.clone());
        }
        planned_variants.push(PlannedVariant {
            rel: rel.clone(),
            base_rel,
            base_ids,
            base2_rel: base2_rel.filter(|_| !base2_ids.is_empty()),
            base2_ids,
            dep_rels,
        });
    }
    // Variant chunk -> family-hub chunks. Every dependency list that names a
    // variant chunk must also name its hub chunks: the runtime decodes the
    // variant grids against the hubs' materialized grids at install time.
    let mut rhs_base_dep = std::collections::BTreeMap::<String, Vec<String>>::new();
    for planned in planned_variants {
        rhs_base_dep.insert(planned.rel.clone(), planned.dep_rels);
        let prep = rhs_preps
            .get_mut(&planned.rel)
            .expect("variant prep exists");
        prep.base_rel = Some(planned.base_rel);
        prep.base_ids = planned.base_ids;
        prep.base2_rel = planned.base2_rel;
        prep.base2_ids = planned.base2_ids;
    }
    // The hub grids a variant decodes against must ship in the hub chunks
    // even when no mission profile reaches them (or the whole hub RHS).
    // TODO: extra grids landing in a hub2 chunk that is itself a planned
    // variant are coded standalone within that chunk (its own base pairing
    // was fixed before the extras arrived); pairing them against hub1 too
    // would shave a little more.
    for (base_rel, extra) in base_extra_ids {
        match rhs_preps.get_mut(&base_rel) {
            Some(prep) => prep.used_sprite_ids.extend(extra),
            None => {
                tracing::info!(
                    rhs = base_rel.as_str(),
                    sprites = extra.len(),
                    "synthesizing sprite-only family-hub chunk"
                );
                rhs_preps.insert(
                    base_rel.clone(),
                    RhsChunkPrep {
                        rhs_data: None,
                        matched_profiles: 0,
                        script_order: Vec::new(),
                        used_sprite_ids: extra,
                        base_rel: None,
                        base_ids: std::collections::BTreeMap::new(),
                        base2_rel: None,
                        base2_ids: std::collections::BTreeMap::new(),
                    },
                );
            }
        }
    }

    // Phase C: assemble the chunk payloads. `encode_grids` dominates this
    // stage, so it runs on the bounded worker pool.
    let built_payloads = compression_pool.install(|| {
        rhs_preps
            .par_iter()
            .map(|(rel, prep)| {
                Ok((
                    rel.clone(),
                    build_rhs_chunk_payload(&holder, dict_remaps.as_deref(), rel, prep)?,
                ))
            })
            .collect::<Vec<Result<(String, ShippingMission)>>>()
    });
    let mut rhs_payloads = std::collections::BTreeMap::<String, ShippingMission>::new();
    for built in built_payloads {
        let (rel, payload) = built?;
        rhs_payloads.insert(rel, payload);
    }

    // Resolve only the terrain and loading art the original runtime can open
    // for this mission. Keep each logical source asset in its own shared
    // payload so missions that reuse a city also reuse one HTTP-cache key.
    let mut encoded_level_assets = std::collections::BTreeMap::<String, Vec<u8>>::new();
    let mut level_asset_payloads = std::collections::BTreeMap::<String, ShippingMission>::new();
    for build in mission_builds.values_mut() {
        for map in &build.map_names {
            // A mission that opens a map always opens its minimap too (the
            // runtime draws both), so both land in ONE shared payload keyed
            // by the `.map` rel: one HTTP fetch / cache key per city and
            // ambiance instead of two. Runtime lookups are by the original
            // asset path inside the payload, so merging is invisible there.
            let map_rel = level_asset_rel_existing(build.ambiance, map, ".map", &in_path)?;
            for ext in [".map", ".min"] {
                let rel = level_asset_rel_existing(build.ambiance, map, ext, &in_path)?;
                let path = in_path(&rel)
                    .ok_or_else(|| anyhow!("resolved shipping level asset disappeared: {rel}"))?;
                let bytes = if let Some(bytes) = encoded_level_assets.get(&rel) {
                    bytes.clone()
                } else {
                    // Minimaps follow the map format: the runtime picture
                    // loader sniffs the JXL signature, so `.min` decodes
                    // through the same path as `.map` with no extra code.
                    let bytes = match opts.map_format.jxl_quality() {
                        Some(quality) => transcode_sixteen_to_jxl(&path, quality)?,
                        None => transcode_sixteen_drop_bzip(&path)?,
                    };
                    encoded_level_assets.insert(rel.clone(), bytes.clone());
                    bytes
                };
                level_asset_payloads
                    .entry(map_rel.clone())
                    .or_default()
                    .raw
                    .insert(rel.to_ascii_lowercase(), bytes);
            }
            build.level_asset_keys.insert(map_rel);
        }
        let rel = format!("Levels/{:02}/{}.pak", build.ambiance, build.proto_filename);
        if let Some(path) = in_path(&rel) {
            let bytes = if let Some(bytes) = encoded_level_assets.get(&rel) {
                bytes.clone()
            } else {
                let bytes = transcode_pak_drop_bzip(&path)?;
                encoded_level_assets.insert(rel.clone(), bytes.clone());
                bytes
            };
            level_asset_payloads
                .entry(rel.clone())
                .or_default()
                .raw
                .insert(rel.to_ascii_lowercase(), bytes);
            build.level_asset_keys.insert(rel);
        }
    }

    // Bake the `import_beam_mes` post-processing into the shipping
    // profile table.  Without this, runtime loaders that consume
    // `dd.profiles` see empty `required_actions` / zero
    // `number_of_beam_mes` — breaking briefing-UI glyphs and
    // auto-gang-selection (see
    // `crates/robin_rs/src/main_entry.rs::load_profiles` for the
    // non-shipping equivalent).
    if let Some(level_dir) = resolve_case_insensitive(&data_in.join("Levels"))
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
    {
        cpf.import_beam_mes(&level_dir);
    } else {
        tracing::warn!(
            "convert_shipping: no Levels/ directory found; shipping profile will lack beam-me data"
        );
    }
    dd.profiles = Some(cpf);

    // Bundle the small-file types the engine opens by exact path — these
    // are the items that would otherwise fan out to hundreds of tiny HTTP
    // requests on wasm and a bunch of syscalls on native.  We deliberately
    // *don't* bundle large files (audio, terrain bitmaps already handled
    // above, cinematics) so the shipping blob stays compact.
    //
    // Keyed by the path the engine passes to `SbFile::open` minus the
    // `Data/` prefix, which matches `asset_fs::bundle_key`.
    const BOOT_FILE_EXTS: &[&str] = &[
        // Fonts
        "bfn", "tfn", "fnt", // Menu / cursor / interface configuration
        "cfg", "ini", // Resource bundles (text tables, cursors, loading screens)
        "res", "pak", "red", // Small shared resource bundles
        "cpf",
    ];
    walk_and_bundle_small(
        &mut dd,
        &data_in,
        &data_in,
        BOOT_FILE_EXTS,
        opts.interface_image_format,
    )?;
    for alt in &locale_dirs {
        walk_and_bundle_small(
            &mut dd,
            &alt.data_dir,
            &alt.data_dir,
            BOOT_FILE_EXTS,
            opts.interface_image_format,
        )?;
    }
    // Keep one zstd stream per RHS rather than one file per sprite. The
    // measurements in docs/COMPRESSION.md show that within-character
    // cross-sprite matching retains the current compression ratio, while
    // shared RHS files avoid duplicating heroes/accessories across missions.
    let mission_dir = data_out.join("missions");
    let rhs_dir = data_out.join("rhs");
    let terrain_dir = data_out.join("terrain");
    let audio_dir = data_out.join("audio");
    fs::create_dir_all(&mission_dir)?;
    fs::create_dir_all(&rhs_dir)?;
    fs::create_dir_all(&terrain_dir)?;
    fs::create_dir_all(&audio_dir)?;
    let encoded_level_assets = compression_pool.install(|| {
        level_asset_payloads
            .into_par_iter()
            .map(|(rel, payload)| {
                let (filename, compressed) = prepare_shipping_payload(
                    &terrain_dir,
                    &rel,
                    &payload,
                    opts.zstd_window_log,
                    opts.resume,
                )?;
                write_prepared_shipping_payload(&terrain_dir, &filename, compressed)?;
                Ok((rel, format!("terrain/{filename}")))
            })
            .collect::<Vec<Result<(String, String)>>>()
    });
    let mut level_asset_files = std::collections::BTreeMap::<String, String>::new();
    for encoded in encoded_level_assets {
        let (rel, filename) = encoded?;
        level_asset_files.insert(rel, filename);
    }
    let encoded_rhs = compression_pool.install(|| {
        rhs_payloads
            .into_par_iter()
            .map(|(rel, payload)| {
                let (filename, compressed) = prepare_shipping_payload(
                    &rhs_dir,
                    &rel,
                    &payload,
                    opts.zstd_window_log,
                    opts.resume,
                )?;
                write_prepared_shipping_payload(&rhs_dir, &filename, compressed)?;
                Ok((rel, filename))
            })
            .collect::<Vec<Result<(String, String)>>>()
    });
    let mut rhs_files = std::collections::BTreeMap::<String, String>::new();
    for encoded in encoded_rhs {
        let (rel, filename) = encoded?;
        let relative = format!("rhs/{filename}");
        rhs_files.insert(rel, relative);
    }
    // A dependency on a family-variant chunk implies its hub chunk(s): the
    // runtime decodes the variant's VQ grids against the hubs' at install
    // (star-2 chunks depend on both hubs).
    let rhs_chunk_files = |rel: &str| -> Result<Vec<String>> {
        let mut chunk_files = Vec::with_capacity(3);
        let file = rhs_files
            .get(rel)
            .ok_or_else(|| anyhow!("missing shipping RHS payload {rel}"))?;
        chunk_files.push(file.clone());
        for base_rel in rhs_base_dep.get(rel).into_iter().flatten() {
            let base_file = rhs_files.get(base_rel).ok_or_else(|| {
                anyhow!("missing shipping RHS family-hub payload {base_rel} (required by {rel})")
            })?;
            chunk_files.push(base_file.clone());
        }
        Ok(chunk_files)
    };
    for (profile_index, requirements) in character_rhs_requirements {
        let mut files = Vec::with_capacity(requirements.len());
        for rel in requirements.keys() {
            files.extend(rhs_chunk_files(rel).with_context(|| {
                format!("character profile {profile_index} RHS dependency {rel}")
            })?);
        }
        files.sort();
        files.dedup();
        dd.character_rhs_files.insert(profile_index, files);
    }
    for rel in saved_world_rhs_requirements.keys() {
        dd.saved_world_rhs_files.extend(
            rhs_chunk_files(rel)
                .with_context(|| format!("saved-world compatibility RHS dependency {rel}"))?,
        );
    }
    dd.saved_world_rhs_files.sort();
    dd.saved_world_rhs_files.dedup();

    // Source-format audio remains in dependency payloads for native builds.
    // Opus browser audio is cataloged as standalone content-addressed files;
    // these payloads then retain only small blocking metadata such as FXG and
    // exclamation DAT files.
    let mut common_audio = ShippingMission::default();
    let sounds_root = data_in.join("Sounds");
    if sounds_root.is_dir() {
        let mut files = Vec::new();
        collect_files_recursive(&sounds_root, &mut files)?;
        files.sort();
        for path in files {
            let relative = path
                .strip_prefix(&sounds_root)
                .expect("collected sound must remain below Sounds")
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if relative.starts_with("menu/") || relative.starts_with("exclamations/") {
                continue;
            }
            insert_shipping_audio(
                &mut common_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "common",
                &format!("Sounds/{relative}"),
                &path,
                AudioKind::Effect,
                opts.audio_format,
            )?;
        }
    }
    let common_audio_file = write_shipping_dependency(
        &audio_dir,
        "common-sfx",
        &common_audio,
        opts.zstd_window_log,
        opts.resume,
    )?;

    // Mission-authored exclamation profiles must resolve completely; ids
    // that only appear in the all-profiles character manifest index may be
    // absent from a trimmed (demo) datadir and are then dropped from the
    // manifest instead of failing the conversion.
    let mission_exclamation_ids: BTreeSet<u32> = mission_builds
        .values()
        .flat_map(|build| build.required_exclamation_ids.iter().copied())
        .collect();
    let mut dropped_exclamation_ids = BTreeSet::<u32>::new();
    let mut required_exclamation_ids: BTreeSet<u32> = mission_builds
        .values()
        .flat_map(|build| build.required_exclamation_ids.iter().copied())
        .chain(
            character_exclamation_ids
                .iter()
                .copied()
                .filter(|id| *id != 0),
        )
        .collect();
    let mut exclamation_metadata = ShippingMission::default();
    let exclamation_root = data_in.join("Sounds/Exclamations");
    if exclamation_root.is_dir() {
        let mut files = Vec::new();
        collect_files_recursive(&exclamation_root, &mut files)?;
        files.sort();
        for path in files {
            // actors.res is already represented authoritatively in
            // `ShippingDatadir::res_files`; voice WAVs are actor chunks below.
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            if extension.eq_ignore_ascii_case("dat") {
                let relative = path
                    .strip_prefix(&data_in)
                    .expect("base exclamation metadata must remain below Data")
                    .to_string_lossy();
                insert_shipping_raw(&mut exclamation_metadata, &relative, &path)?;
            }
        }
    }
    // A localized install may put actor tables in its locale overlay rather
    // than the base Exclamations directory. Ensure every referenced table is
    // mounted under the logical path used by the runtime.
    for exclamation_id in &required_exclamation_ids {
        let dat_rel = format!(
            "Sounds/Exclamations/{}",
            exclamation_dat_filename(*exclamation_id)
        );
        if let Some(path) = in_path(&dat_rel) {
            insert_shipping_raw(&mut exclamation_metadata, &dat_rel, &path)?;
        }
    }
    let actors_res_path = in_path("Sounds/Exclamations/actors.res")
        .ok_or_else(|| anyhow!("Sounds/Exclamations/actors.res missing"))?;
    let mut actors_res = ResourceManager::new();
    actors_res.attach_resource_file(&actors_res_path.to_string_lossy())?;
    let mut actor_samples = std::collections::BTreeMap::<u32, Vec<(String, PathBuf)>>::new();
    let mut sample_profile_counts = std::collections::BTreeMap::<String, usize>::new();
    'ids: for &exclamation_id in &required_exclamation_ids {
        let strict = mission_exclamation_ids.contains(&exclamation_id);
        let dat_filename = exclamation_dat_filename(exclamation_id);
        let dat_rel = format!("Sounds/Exclamations/{dat_filename}");
        let dat_path = match in_path(&dat_rel) {
            Some(path) => path,
            None if strict => bail!(
                "required exclamation profile {exclamation_id:#010x} is missing metadata {dat_rel}"
            ),
            None => {
                tracing::warn!(
                    "exclamation profile {exclamation_id:#010x} has no metadata {dat_rel} in this datadir; omitting from manifest index"
                );
                dropped_exclamation_ids.insert(exclamation_id);
                continue 'ids;
            }
        };
        let dat = fs::read(&dat_path)
            .with_context(|| format!("read exclamation metadata {}", dat_path.display()))?;
        let prefix_id = exclamation_id & 0xffff_0000;
        let (table_id, exclamations) =
            robin_engine::sound_cache::parse_exclamation_file(&dat, prefix_id)
                .map_err(|error| anyhow!("parse exclamation metadata {dat_filename}: {error}"))?;
        let variant_indices: BTreeSet<u32> = exclamations
            .into_iter()
            .flat_map(|(_, variants)| variants)
            .collect();
        let mut samples = Vec::with_capacity(variant_indices.len());
        for variant_index in variant_indices {
            let sample = actors_res
                .get_sample(table_id as i32, variant_index as usize)
                .with_context(|| {
                    format!("resolve exclamation {exclamation_id:#010x} variant {variant_index}")
                })?
                .replace('\\', "/");
            let sample_rel = format!("Sounds/Exclamations/{sample}");
            let sample_path = match in_path(&sample_rel) {
                Some(path) => path,
                None if strict => bail!(
                    "required exclamation profile {exclamation_id:#010x} variant {variant_index} references missing sample {sample_rel}"
                ),
                None => {
                    tracing::warn!(
                        "exclamation profile {exclamation_id:#010x} references missing sample {sample_rel} in this datadir; omitting from manifest index"
                    );
                    dropped_exclamation_ids.insert(exclamation_id);
                    continue 'ids;
                }
            };
            samples.push((sample_rel.clone(), sample_path));
        }
        for (sample_rel, _) in &samples {
            *sample_profile_counts.entry(sample_rel.clone()).or_default() += 1;
        }
        actor_samples.insert(exclamation_id, samples);
    }
    required_exclamation_ids.retain(|id| !dropped_exclamation_ids.contains(id));

    // A handful of generic samples (notably x_empty.wav) are referenced by
    // multiple actor tables. Store those once in the shared exclamation
    // payload rather than downloading duplicate bytes or mounting duplicate
    // VFS keys from several actor chunks.
    for (sample_rel, profile_count) in &sample_profile_counts {
        if *profile_count > 1 {
            let sample_path = in_path(sample_rel).ok_or_else(|| {
                anyhow!("shared exclamation sample disappeared during conversion: {sample_rel}")
            })?;
            insert_shipping_audio(
                &mut exclamation_metadata,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "voice-shared",
                sample_rel,
                &sample_path,
                AudioKind::Voice,
                opts.audio_format,
            )?;
        }
    }
    let exclamation_metadata_file = write_shipping_dependency(
        &audio_dir,
        "exclamation-metadata",
        &exclamation_metadata,
        opts.zstd_window_log,
        opts.resume,
    )?;

    let mut actor_voice_files = std::collections::BTreeMap::<u32, String>::new();
    for exclamation_id in required_exclamation_ids {
        let mut actor_audio = ShippingMission::default();
        let samples = actor_samples.remove(&exclamation_id).ok_or_else(|| {
            anyhow!("missing resolved sample set for exclamation profile {exclamation_id:#010x}")
        })?;
        for (sample_rel, sample_path) in samples {
            if sample_profile_counts.get(&sample_rel).copied().unwrap_or(0) == 1 {
                insert_shipping_audio(
                    &mut actor_audio,
                    &mut dd.audio_assets,
                    &audio_assets_dir,
                    &format!("voice-{exclamation_id:08x}"),
                    &sample_rel,
                    &sample_path,
                    AudioKind::Voice,
                    opts.audio_format,
                )?;
            }
        }
        if let Some(relative) = write_shipping_dependency(
            &audio_dir,
            &format!("voice-{exclamation_id:08x}"),
            &actor_audio,
            opts.zstd_window_log,
            opts.resume,
        )? {
            actor_voice_files.insert(exclamation_id, relative);
        }
    }

    for (profile_index, exclamation_id) in character_exclamation_ids.into_iter().enumerate() {
        let profile_index =
            u32::try_from(profile_index).context("character profile index exceeds u32")?;
        let files = actor_voice_files
            .get(&exclamation_id)
            .cloned()
            .into_iter()
            .collect();
        dd.character_audio_files.insert(profile_index, files);
        if exclamation_id != 0 && !dropped_exclamation_ids.contains(&exclamation_id) {
            dd.character_exclamation_ids
                .insert(profile_index, exclamation_id);
        }
    }

    let encoded_missions = compression_pool.install(|| {
        mission_builds
            .into_par_iter()
            .map(|(mission_name, build)| {
                let ShippingMissionBuild {
                    payload,
                    required_rhs_profiles,
                    required_exclamation_ids,
                    music_names,
                    dialogue_samples,
                    level_asset_keys,
                    forest_level,
                    ..
                } = build;
                let (filename, compressed) = prepare_shipping_payload(
                    &mission_dir,
                    &mission_name,
                    &payload,
                    opts.zstd_window_log,
                    opts.resume,
                )?;
                let compressed_len =
                    write_prepared_shipping_payload(&mission_dir, &filename, compressed)?;
                Ok((
                    mission_name,
                    filename,
                    compressed_len,
                    required_rhs_profiles,
                    required_exclamation_ids,
                    music_names,
                    dialogue_samples,
                    level_asset_keys,
                    forest_level,
                ))
            })
            .collect::<Vec<Result<_>>>()
    });
    for encoded in encoded_missions {
        let (
            mission_name,
            filename,
            compressed_len,
            required_rhs_profiles,
            required_exclamation_ids,
            music_names,
            dialogue_samples,
            level_asset_keys,
            forest_level,
        ) = encoded?;
        let relative = format!("missions/{filename}");
        let mut files = vec![relative.clone()];
        for rel in level_asset_keys {
            let file = level_asset_files.get(&rel).ok_or_else(|| {
                anyhow!("shipping mission {mission_name} requires missing terrain payload {rel}")
            })?;
            files.push(file.clone());
        }
        for rel in required_rhs_profiles.keys() {
            files.extend(
                rhs_chunk_files(rel)
                    .with_context(|| format!("shipping mission {mission_name} RHS dependency"))?,
            );
        }
        if let Some(file) = common_audio_file.as_ref() {
            files.push(file.clone());
        }
        if let Some(file) = exclamation_metadata_file.as_ref() {
            files.push(file.clone());
        }
        for exclamation_id in &required_exclamation_ids {
            if let Some(file) = actor_voice_files.get(exclamation_id) {
                files.push(file.clone());
            }
        }
        let mut dialogue_audio = ShippingMission::default();
        for sample_rel in &dialogue_samples {
            let sample_path = in_path(sample_rel).ok_or_else(|| {
                anyhow!(
                    "shipping mission {mission_name} references missing dialogue sample {sample_rel}"
                )
            })?;
            insert_shipping_audio(
                &mut dialogue_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                &format!("dialogue-{}", shipping_file_stem(&mission_name)),
                sample_rel,
                &sample_path,
                AudioKind::Voice,
                opts.audio_format,
            )?;
        }
        if let Some(file) = write_shipping_dependency(
            &audio_dir,
            "mission-dialogue",
            &dialogue_audio,
            opts.zstd_window_log,
            opts.resume,
        )? {
            files.push(file);
        }
        let mut music_audio = ShippingMission::default();
        for name in &music_names {
            // SoundManager requests `.wav`, but the Linux release ships Ogg
            // and the audio backend deliberately falls back between them.
            // Preserve whichever real file the source datadir provides.
            let (relative, path) = ["wav", "ogg"]
                .into_iter()
                .find_map(|extension| {
                    let relative = format!("Musics/{name}.{extension}");
                    in_path(&relative).map(|path| (relative, path))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "shipping mission {mission_name} references missing music Musics/{name}.{{wav,ogg}}"
                    )
                })?;
            insert_shipping_audio(
                &mut music_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "music",
                &relative,
                &path,
                AudioKind::Music,
                opts.audio_format,
            )?;
        }
        if let Some(file) = write_shipping_dependency(
            &audio_dir,
            "mission-music",
            &music_audio,
            opts.zstd_window_log,
            opts.resume,
        )? {
            files.push(file);
        }
        tracing::info!(
            mission = mission_name,
            bytes = compressed_len,
            dependencies = files.len(),
            file = relative,
            "wrote shipping mission payload"
        );
        dd.mission_exclamation_ids.insert(
            mission_name.clone(),
            required_exclamation_ids.iter().copied().collect(),
        );
        files.sort();
        files.dedup();
        dd.missions.insert(
            mission_name,
            ShippingMissionRef {
                forest_level,
                files,
            },
        );
    }

    bundle_grouped_audio(&mut dd, &data_out)?;

    // Serialize + compress with the configured window log.
    let out_file = data_out.join("datadir.bin");
    let blob = robin_assets::shipping_datadir::encode_native(&dd);
    let compressed =
        robin_assets::shipping_datadir::zstd_compress_with_window(&blob, opts.zstd_window_log)?;
    fs::write(&out_file, compressed).with_context(|| format!("write {}", out_file.display()))?;
    tracing::info!(
        "wrote {} (windowLog={}, map={:?}, audio={:?})",
        out_file.display(),
        opts.zstd_window_log,
        opts.map_format,
        opts.audio_format
    );
    Ok(())
}

fn shipping_file_stem(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn shipping_payload_filename(name: &str, window_log: u32, compressed: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(compressed);
    let hash: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "{}-w{window_log}-{hash}.rhmission.zst",
        shipping_file_stem(name)
    )
}

fn encode_shipping_payload(payload: &ShippingMission, window_log: u32) -> Result<Vec<u8>> {
    let encoded = robin_assets::shipping_datadir::encode_mission_native(payload);
    robin_assets::shipping_datadir::zstd_compress_with_window(&encoded, window_log)
}

fn write_prepared_shipping_payload(
    output_dir: &Path,
    filename: &str,
    compressed: Option<Vec<u8>>,
) -> Result<usize> {
    let path = output_dir.join(filename);
    if let Some(compressed) = compressed {
        let len = compressed.len();
        fs::write(&path, compressed).with_context(|| format!("write {}", path.display()))?;
        Ok(len)
    } else {
        Ok(fs::metadata(&path)
            .with_context(|| format!("stat reused payload {}", path.display()))?
            .len() as usize)
    }
}

/// Return a validated existing content filename, or freshly compressed bytes
/// and their content-addressed filename. Reuse compares the complete decoded
/// native-bitcode payload, so an interrupted run cannot accidentally mix
/// schemas, source data, or converter options.
fn prepare_shipping_payload(
    output_dir: &Path,
    label: &str,
    payload: &ShippingMission,
    window_log: u32,
    resume: bool,
) -> Result<(String, Option<Vec<u8>>)> {
    if resume {
        let prefix = format!("{}-w{window_log}-", shipping_file_stem(label));
        let expected = robin_assets::shipping_datadir::encode_mission_native(payload);
        let mut candidates = fs::read_dir(output_dir)
            .with_context(|| format!("read_dir {}", output_dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&prefix) && name.ends_with(".rhmission.zst")
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort();
        for path in candidates {
            let compressed = match fs::read(&path) {
                Ok(compressed) => compressed,
                Err(_) => continue,
            };
            let decoded =
                match robin_assets::shipping_datadir::decode_mission_compressed(&compressed) {
                    Ok(decoded) => decoded,
                    Err(_) => continue,
                };
            if robin_assets::shipping_datadir::encode_mission_native(&decoded) == expected {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("candidate shipping filename was valid UTF-8")
                    .to_owned();
                tracing::info!(label, filename, "reused validated shipping payload");
                return Ok((filename, None));
            }
        }
    }

    let compressed = encode_shipping_payload(payload, window_log)?;
    let filename = shipping_payload_filename(label, window_log, &compressed);
    Ok((filename, Some(compressed)))
}

fn collect_files_recursive(src: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn insert_shipping_raw(payload: &mut ShippingMission, relative: &str, path: &Path) -> Result<()> {
    let relative = relative.replace('\\', "/").to_ascii_lowercase();
    let bytes = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    if let Some(previous) = payload.raw.get(&relative) {
        if previous != &bytes {
            bail!("conflicting shipping audio sources for {relative}");
        }
    } else {
        payload.raw.insert(relative, bytes);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum AudioKind {
    Voice,
    Effect,
    Music,
}

impl AudioKind {
    fn bitrate_kbps(self) -> u32 {
        match self {
            Self::Voice => 24,
            Self::Effect => 48,
            // 64 -> 48 kbit/s together with switching music to the lossless
            // remaster sources (see `music_lossless_source`): encoding from
            // a clean master at 48k beats encoding the shipped lossy WAVs
            // at 64k, and drops ~25% of the music bytes.
            Self::Music => 48,
        }
    }

    fn opus_application(self) -> &'static str {
        match self {
            Self::Voice => "voip",
            Self::Effect | Self::Music => "audio",
        }
    }
}

/// Logical bundle groups recorded during catalog construction, keyed by the
/// content-addressed asset file. A file referenced from several groups lands
/// in the "shared" bundle (see `bundle_grouped_audio`).
static AUDIO_ASSET_GROUPS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn insert_shipping_audio(
    payload: &mut ShippingMission,
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    path: &Path,
    kind: AudioKind,
    format: AudioFormat,
) -> Result<()> {
    let is_audio = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("ogg")
        });
    if !is_audio {
        return insert_shipping_raw(payload, relative, path);
    }

    let source = fs::read(path).with_context(|| format!("read audio {}", path.display()))?;
    let duration_ms = robin_rs::audio_backend::wav_duration_ms(&source).ok_or_else(|| {
        anyhow!(
            "cannot derive authoritative audio duration for {}",
            path.display()
        )
    })?;
    match format {
        AudioFormat::Source => {
            let relative = relative.replace('\\', "/").to_ascii_lowercase();
            if let Some(previous) = payload.raw.get(&relative) {
                if previous != &source {
                    bail!("conflicting shipping audio sources for {relative}");
                }
            } else {
                payload.raw.insert(relative.clone(), source);
            }
            insert_audio_duration(&mut payload.audio_durations_ms, relative, duration_ms)
        }
        AudioFormat::Opus => {
            let logical = standalone_audio_logical_key(relative);
            if let Some(existing) = catalog.get(&logical) {
                if existing.duration_ms != duration_ms {
                    bail!(
                        "conflicting source durations for standalone shipping audio {logical}: {} vs {duration_ms}",
                        existing.duration_ms
                    );
                }
                return Ok(());
            }
            // Music encodes from the lossless remaster drop when one exists;
            // the catalog duration above stays derived from the GAME source,
            // so timing-deterministic tables are unaffected by small length
            // differences in the masters.
            let encode_source = if matches!(kind, AudioKind::Music) {
                music_lossless_source(path)
            } else {
                None
            };
            let bytes = transcode_audio_to_opus(encode_source.as_deref().unwrap_or(path), kind)?;
            insert_standalone_audio(catalog, assets_dir, group, relative, &bytes, duration_ms)
        }
    }
}

/// Resolve the higher-quality lossless master for a music track, when the
/// optional `datadirs/music-rhmods-lossless` drop is present. Its
/// `mapping.json` maps game file names (under `DATA/Musics`) to remaster
/// file names; entries mapped to `null` have no clean source and fall back
/// to the game WAV. Matching is by file name, case-insensitive.
fn music_lossless_source(game_path: &Path) -> Option<PathBuf> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    static MAPPING: OnceLock<BTreeMap<String, PathBuf>> = OnceLock::new();
    let mapping = MAPPING.get_or_init(|| {
        let root = Path::new("datadirs/music-rhmods-lossless");
        let mapping_path = root.join("mapping.json");
        let json: serde_json::Value = match fs::read_to_string(&mapping_path)
            .map_err(anyhow::Error::from)
            .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from))
        {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!(
                    "no lossless music mapping at {} ({e:#}); music encodes from game sources",
                    mapping_path.display()
                );
                return BTreeMap::new();
            }
        };
        let lossless_root = json
            .get("lossless_root")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut resolved = BTreeMap::new();
        let Some(pairs) = json.get("game_to_lossless").and_then(|v| v.as_object()) else {
            tracing::warn!("lossless music mapping has no game_to_lossless object");
            return resolved;
        };
        for (game_name, lossless_name) in pairs {
            let Some(lossless_name) = lossless_name.as_str() else {
                continue; // null: no clean source for this track
            };
            // The drop has been seen both with and without the
            // `lossless_root` subdirectory; accept either layout.
            let candidates = [
                root.join(lossless_root).join(lossless_name),
                root.join(lossless_name),
            ];
            match candidates.into_iter().find(|p| p.is_file()) {
                Some(path) => {
                    resolved.insert(game_name.to_ascii_lowercase(), path);
                }
                None => tracing::warn!(
                    "lossless music mapping names missing file {lossless_name} for {game_name}"
                ),
            }
        }
        tracing::info!(
            tracks = resolved.len(),
            "music will encode from lossless remaster sources"
        );
        resolved
    });
    let name = game_path.file_name()?.to_str()?.to_ascii_lowercase();
    mapping.get(&name).cloned()
}

fn insert_standalone_audio(
    catalog: &mut std::collections::BTreeMap<String, ShippingAudioAsset>,
    assets_dir: &Path,
    group: &str,
    relative: &str,
    bytes: &[u8],
    duration_ms: u32,
) -> Result<()> {
    let encoded_size =
        u32::try_from(bytes.len()).context("standalone Opus asset exceeds u32 byte length")?;
    let logical = standalone_audio_logical_key(relative);
    let filename = standalone_audio_filename(bytes);
    let output = assets_dir.join(&filename);
    if output.exists() {
        let existing = fs::read(&output)
            .with_context(|| format!("read existing audio asset {}", output.display()))?;
        if existing != bytes {
            bail!("content-addressed audio collision at {}", output.display());
        }
    } else {
        fs::write(&output, bytes)
            .with_context(|| format!("write audio asset {}", output.display()))?;
    }
    let file = format!("audio/assets/{filename}");
    AUDIO_ASSET_GROUPS
        .lock()
        .expect("audio group recorder poisoned")
        .entry(file.clone())
        .or_default()
        .insert(group.to_owned());
    let asset = ShippingAudioAsset {
        file,
        encoded_size,
        duration_ms,
        bundle_offset: None,
    };
    match catalog.entry(logical) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(asset);
        }
        std::collections::btree_map::Entry::Occupied(entry) if entry.get() == &asset => {}
        std::collections::btree_map::Entry::Occupied(entry) => {
            bail!(
                "conflicting standalone shipping audio for {}: {:?} vs {asset:?}",
                entry.key(),
                entry.get()
            );
        }
    }
    Ok(())
}

fn standalone_audio_logical_key(relative: &str) -> String {
    Path::new(&robin_util::asset_fs::bundle_key(Path::new(relative)))
        .with_extension("opus")
        .to_string_lossy()
        .replace('\\', "/")
}

fn insert_audio_duration(
    durations: &mut std::collections::BTreeMap<String, u32>,
    relative: String,
    duration_ms: u32,
) -> Result<()> {
    match durations.entry(relative) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(duration_ms);
        }
        std::collections::btree_map::Entry::Occupied(entry) if *entry.get() == duration_ms => {}
        std::collections::btree_map::Entry::Occupied(entry) => {
            bail!(
                "conflicting source durations for shipping audio {}: {} vs {duration_ms}",
                entry.key(),
                entry.get()
            );
        }
    }
    Ok(())
}

/// Assets larger than this stay standalone files (music, long ambience):
/// they are few, individually worth an HTTP request, and bundling them
/// would force multi-MB downloads for one sound.
const AUDIO_BUNDLE_MAX_MEMBER: u32 = 262_144;

/// Concatenate small catalog assets into one file per logical group
/// (recorded in [`AUDIO_ASSET_GROUPS`] during catalog construction; a file
/// referenced by several groups moves to the "shared" bundle). Rewrites the
/// catalog entries to (bundle file, offset) and deletes the standalone
/// files, so the browser fetches one request per group instead of ~2,000
/// tiny ones. Deterministic: members concatenate in content-hash order and
/// the bundle name is content-addressed.
fn bundle_grouped_audio(
    dd: &mut robin_assets::shipping_datadir::ShippingDatadir,
    data_out: &Path,
) -> Result<()> {
    use sha2::{Digest as _, Sha256};
    use std::collections::{BTreeMap, BTreeSet};
    let groups_by_file = std::mem::take(
        &mut *AUDIO_ASSET_GROUPS
            .lock()
            .expect("audio group recorder poisoned"),
    );
    if groups_by_file.is_empty() {
        return Ok(());
    }
    // file -> (logical keys referencing it, encoded size)
    let mut file_refs = BTreeMap::<String, (Vec<String>, u32)>::new();
    for (logical, asset) in &dd.audio_assets {
        if asset.bundle_offset.is_some() {
            bail!("audio asset {logical} is already bundled; bundling must run once");
        }
        let entry = file_refs
            .entry(asset.file.clone())
            .or_insert_with(|| (Vec::new(), asset.encoded_size));
        if entry.1 != asset.encoded_size {
            bail!("conflicting encoded sizes recorded for {}", asset.file);
        }
        entry.0.push(logical.clone());
    }
    let mut members_by_group = BTreeMap::<String, Vec<String>>::new();
    for (file, (_, size)) in &file_refs {
        if *size >= AUDIO_BUNDLE_MAX_MEMBER {
            continue;
        }
        let groups = groups_by_file
            .get(file)
            .cloned()
            .unwrap_or_else(BTreeSet::new);
        let group = match groups.len() {
            0 => bail!("catalog file {file} was never recorded in a bundle group"),
            1 => groups.into_iter().next().expect("len checked"),
            _ => "shared".to_owned(),
        };
        members_by_group
            .entry(group)
            .or_default()
            .push(file.clone());
    }
    let bundles_dir = data_out.join("audio/bundles");
    fs::create_dir_all(&bundles_dir)?;
    let (mut bundled_files, mut bundle_count, mut bundled_bytes) = (0usize, 0usize, 0u64);
    for (group, members) in members_by_group {
        // BTreeMap iteration already sorted members by content-hash name.
        let mut bytes = Vec::new();
        let mut offsets = Vec::with_capacity(members.len());
        for file in &members {
            let member_bytes = fs::read(data_out.join(file))
                .with_context(|| format!("read bundle member {file}"))?;
            let expected = file_refs[file].1 as usize;
            if member_bytes.len() != expected {
                bail!(
                    "bundle member {file} is {} bytes on disk but cataloged as {expected}",
                    member_bytes.len()
                );
            }
            offsets.push(u32::try_from(bytes.len()).context("audio bundle exceeds u32")?);
            bytes.extend_from_slice(&member_bytes);
        }
        let digest = Sha256::digest(&bytes);
        let hash: String = digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let bundle_rel = format!("audio/bundles/{}-{hash}.bin", shipping_file_stem(&group));
        fs::write(data_out.join(&bundle_rel), &bytes)
            .with_context(|| format!("write {bundle_rel}"))?;
        bundle_count += 1;
        bundled_bytes += bytes.len() as u64;
        for (file, offset) in members.iter().zip(offsets) {
            for logical in &file_refs[file].0 {
                let asset = dd
                    .audio_assets
                    .get_mut(logical)
                    .expect("logical key came from the catalog");
                asset.file = bundle_rel.clone();
                asset.bundle_offset = Some(offset);
            }
            fs::remove_file(data_out.join(file))
                .with_context(|| format!("remove bundled standalone {file}"))?;
            bundled_files += 1;
        }
    }
    tracing::info!(
        bundles = bundle_count,
        bundled_files,
        bundled_bytes,
        standalone_left = file_refs.len() - bundled_files,
        "grouped small audio assets into logical bundles"
    );
    Ok(())
}

fn standalone_audio_filename(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(bytes);
    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hash}.opus")
}

/// Encode through FFmpeg's mature libopus integration, then remux the packets
/// with a fixed Ogg stream serial and vendor packet. FFmpeg randomizes Ogg
/// serials, which would otherwise make content-addressed shipping chunks and
/// `--resume` nondeterministic even when the encoded Opus packets are equal.
fn transcode_audio_to_opus(source_path: &Path, kind: AudioKind) -> Result<Vec<u8>> {
    use std::io::Cursor;

    let bitrate = format!("{}k", kind.bitrate_kbps());
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(source_path)
        .args([
            "-map_metadata",
            "-1",
            "-vn",
            "-c:a",
            "libopus",
            "-b:a",
            &bitrate,
            "-vbr",
            "on",
            "-compression_level",
            "10",
            "-frame_duration",
            "20",
            "-application",
            kind.opus_application(),
            "-f",
            "ogg",
            "pipe:1",
        ])
        .output()
        .context("run ffmpeg with libopus support (is ffmpeg installed?)")?;
    if !output.status.success() {
        bail!(
            "ffmpeg Opus encode failed for {} ({}): {}",
            source_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut reader = ogg::PacketReader::new(Cursor::new(output.stdout));
    let mut packets = Vec::new();
    while let Some(packet) = reader
        .read_packet()
        .with_context(|| format!("parse ffmpeg Ogg output for {}", source_path.display()))?
    {
        packets.push(packet);
    }
    if packets
        .first()
        .is_none_or(|packet| !packet.data.starts_with(b"OpusHead"))
    {
        bail!(
            "ffmpeg produced non-Opus Ogg output for {}",
            source_path.display()
        );
    }
    if packets.len() < 3 {
        bail!(
            "ffmpeg produced incomplete Opus stream for {}",
            source_path.display()
        );
    }
    packets[1].data = deterministic_opus_tags();

    let mut remuxed = Cursor::new(Vec::new());
    {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut writer = PacketWriter::new(&mut remuxed);
        for packet in packets {
            let absgp = packet.absgp_page();
            let end = if packet.last_in_stream() {
                PacketWriteEndInfo::EndStream
            } else if packet.last_in_page() {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            writer
                .write_packet(packet.data, 0x5248_4f50, end, absgp)
                .with_context(|| {
                    format!("write deterministic Ogg for {}", source_path.display())
                })?;
        }
    }
    Ok(remuxed.into_inner())
}

fn deterministic_opus_tags() -> Vec<u8> {
    const VENDOR: &[u8] = b"robinhood-web-shipping";
    let mut tags = Vec::with_capacity(16 + VENDOR.len());
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    tags.extend_from_slice(VENDOR);
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags
}

fn write_shipping_dependency(
    output_dir: &Path,
    label: &str,
    payload: &ShippingMission,
    window_log: u32,
    resume: bool,
) -> Result<Option<String>> {
    if payload.raw.is_empty() {
        return Ok(None);
    }
    let (filename, compressed) =
        prepare_shipping_payload(output_dir, label, payload, window_log, resume)?;
    let path = output_dir.join(&filename);
    let compressed_len = if let Some(compressed) = compressed {
        let compressed_len = compressed.len();
        fs::write(&path, compressed).with_context(|| format!("write {}", path.display()))?;
        compressed_len
    } else {
        fs::metadata(&path)
            .with_context(|| format!("stat reused payload {}", path.display()))?
            .len() as usize
    };
    tracing::info!(
        label,
        files = payload.raw.len(),
        bytes = compressed_len,
        "wrote shipping audio dependency"
    );
    Ok(Some(format!("audio/{filename}")))
}

fn exclamation_dat_filename(exclamation_id: u32) -> String {
    let suffix: String = exclamation_id
        .to_le_bytes()
        .into_iter()
        .filter(|byte| *byte != 0)
        .map(char::from)
        .collect();
    format!("actor{suffix}.dat")
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

/// Files represented authoritatively by parsed shipping fields, plus a legacy
/// launcher slideshow that the Rust runtime never consumes. Keeping these raw
/// copies doubles their decoded wasm heap cost without providing a fallback.
fn omit_boot_raw(path: &str) -> bool {
    matches!(
        path.replace('\\', "/").to_ascii_lowercase().as_str(),
        "interface/default.res"
            | "text/level.res"
            | "text/actors.res"
            | "sounds/exclamations/actors.res"
            | "configuration/profile.cpf"
            | "configuration/keyset1.cfg"
            | "configuration/keyset2.cfg"
            | "interface/slideshow_in.pak"
    )
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

/// RGBA with the sprite transparency key mapped to alpha; effort 9 because
/// interface art is small and encoded once.
fn transcode_picture_to_jxl_rgba_keyed(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    use robin_assets::frame_holder::TRANSPARENT_COLOR_16;

    let rgba = pic.to_rgba8888(Some(TRANSPARENT_COLOR_16));
    transcode_pixels_to_jxl(pic, rgba, png::ColorType::Rgba, quality, 9)
}

/// Opaque RGB (maps); effort 7 — effort 9 did not produce a meaningful size
/// win for this content and is much slower.
fn transcode_picture_to_jxl(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    let rgb = picture_to_rgb888(pic)?;
    transcode_pixels_to_jxl(pic, rgb, png::ColorType::Rgb, quality, 7)
}

/// Encode raw pixel data to JXL by piping a minimal PNG through the `cjxl`
/// CLI (PNG on stdin → JXL on stdout). `quality = None` → lossless modular
/// (`-d 0 --modular=1`); `Some(q)` → VarDCT at quality `q`.
///
/// stdin is fed from a scoped thread while the parent drains stdout/stderr,
/// so a picture larger than the pipe buffer cannot deadlock the exchange.
fn transcode_pixels_to_jxl(
    pic: &Picture,
    pixels: Vec<u8>,
    color: png::ColorType,
    quality: Option<u8>,
    effort: u8,
) -> Result<Vec<u8>> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, pic.width as u32, pic.height as u32);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().context("png header")?;
        w.write_image_data(&pixels).context("png data")?;
    }

    let mut cmd = Command::new("cjxl");
    let effort = effort.to_string();
    if let Some(q) = quality {
        cmd.args(["-q", &q.to_string(), "-e", &effort, "-", "-"]);
    } else {
        cmd.args(["-d", "0", "--modular=1", "-e", &effort, "-", "-"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn cjxl (is it installed?)")?;
    let mut stdin = child.stdin.take().expect("cjxl stdin was requested piped");
    let (out, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let result = stdin.write_all(&png_bytes);
            // Explicit drop closes the pipe so cjxl sees EOF.
            drop(stdin);
            result
        });
        // `wait_with_output` drains stdout and stderr concurrently while
        // the writer thread feeds stdin.
        let out = child.wait_with_output().context("cjxl wait");
        let write_result = writer.join().expect("cjxl stdin writer thread panicked");
        (out, write_result)
    });
    let out = out?;
    if !out.status.success() {
        bail!(
            "cjxl failed: exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    write_result.context("write PNG to cjxl")?;
    Ok(out.stdout)
}

fn picture_to_rgb888(pic: &Picture) -> Result<Vec<u8>> {
    use robin_assets::picture::PixelFormat;

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
            for px in rgba.as_chunks::<4>().0 {
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
        if omit_boot_raw(&rel) {
            continue;
        }
        if ext == "red" {
            // Every mission descriptor was parsed into `dd.red_files` above;
            // raw fallback would duplicate it and undermine that invariant.
            continue;
        }
        if ext == "pak" && rel.starts_with("levels/") && !exts.contains(&"rhm") {
            // Split shipping puts the selected level's loading screen in its
            // mission payload; fetching all level paks at boot defeats that.
            continue;
        }
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
    use robin_assets::picture::SixteenPacking;
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
    use robin_assets::picture::SixteenPacking;
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
    use robin_assets::picture::SixteenPacking;
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
    use robin_assets::picture::SixteenPacking;
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
    sprite: &robin_engine::level_data::RawSpriteRef,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    if sprite.frame_profile_name.is_empty() || sprite.profile_name.is_empty() {
        return;
    }
    let rel = animation_rhs_rel_existing(ambiance, &sprite.frame_profile_name, in_path);
    if in_path(&rel).is_some() {
        add_required_rhs_rel(required, rel, &sprite.profile_name);
    } else {
        // TODO: Determine why a few stock level records name an RHS that is
        // absent from every original animation directory.
        tracing::warn!("source animation RHS is absent: {rel}");
    }
}

fn animation_rhs_rel_existing(
    ambiance: u32,
    file: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> String {
    // Mission headers store the original AMBIANCE_* bit values, not a
    // zero-based enum. Keep this identical to engine::Ambiance::from_raw()
    // followed by to_sprite_ambiance(): attack/custom ambiances use Day RHS.
    let dir = match ambiance {
        2 => "Fog",
        4 => "Night",
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

fn level_ambiance_directory(ambiance: u32) -> Result<&'static str> {
    match ambiance {
        1 => Ok("Day"),
        2 => Ok("Fog"),
        4 => Ok("Night"),
        8 => Ok("Attack"),
        16 => Ok("Custom1"),
        32 => Ok("Custom2"),
        64 => Ok("Custom3"),
        128 => Ok("Custom4"),
        _ => bail!("unknown mission ambiance bit value {ambiance}"),
    }
}

/// Resolve a map or minimap exactly like the original engine: selected
/// ambiance first, then Day, then the Levels root. Map and minimap are
/// resolved independently because installs may place their fallbacks at
/// different levels.
fn level_asset_rel_existing(
    ambiance: u32,
    map: &str,
    extension: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<String> {
    let directory = level_ambiance_directory(ambiance)?;
    let mut candidates = vec![format!("Levels/{directory}/{map}{extension}")];
    if directory != "Day" {
        candidates.push(format!("Levels/Day/{map}{extension}"));
    }
    candidates.push(format!("Levels/{map}{extension}"));
    candidates
        .into_iter()
        .find(|candidate| in_path(candidate).is_some())
        .ok_or_else(|| {
            anyhow!(
                "required level asset {map}{extension} is absent from {directory}, Day, and Levels root"
            )
        })
}

/// Cap on sampled tiles per family-base proxy measurement: enough for a
/// stable entropy estimate, small enough to keep conversion fast.
const FAMILY_PROXY_TILE_CAP: u64 = 1_500_000;

/// Conditional entropy in *total bits over all tiles* from 24-bit joint
/// (ctx<<12|sym) counts, scaled from the sampled tile count to `full_tiles`.
fn family_proxy_bits(
    joint: &std::collections::HashMap<u32, u32>,
    ctx_totals: &std::collections::HashMap<u16, u32>,
    sampled: u64,
    full_tiles: u64,
) -> f64 {
    if sampled == 0 {
        return 0.0;
    }
    let mut bits = 0.0f64;
    for (&k, &n) in joint {
        let ctx_total = ctx_totals[&((k >> 12) as u16)] as f64;
        bits -= n as f64 * (n as f64 / ctx_total).log2();
    }
    bits / sampled as f64 * full_tiles as f64
}

/// Resolve a family hub's chunk rel (reusing an existing prep's spelling when
/// one matches case-insensitively) and ensure its full-profile script order is
/// available — either from its prep or loaded into `loaded_orders`.
fn resolve_family_hub_rel(
    prep_rels: &[String],
    rhs_preps: &std::collections::BTreeMap<String, RhsChunkPrep>,
    loaded_orders: &mut std::collections::BTreeMap<String, Vec<u32>>,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
    hub_name: &str,
    variant_rel: &str,
) -> Result<String> {
    let disk_rel = format!("Characters/{hub_name}.rhs");
    let hub_rel = prep_rels
        .iter()
        .find(|rel| rel.eq_ignore_ascii_case(&disk_rel))
        .cloned()
        .unwrap_or(disk_rel);
    if !rhs_preps.contains_key(&hub_rel) && !loaded_orders.contains_key(&hub_rel) {
        let path = in_path(&hub_rel).ok_or_else(|| {
            anyhow!(
                "family hub RHS {hub_rel} (for variant {variant_rel}) is missing from the datadir"
            )
        })?;
        let (_, profiles) =
            sprite_script::SpriteScriptor::load_all_profiles(&path.to_string_lossy())
                .map_err(|error| anyhow!("rhs {hub_rel}: {error}"))?;
        let mut order = Vec::new();
        for (_, info) in &profiles {
            for script in info.scripts.iter() {
                order.extend_from_slice(&script.frame_ids);
            }
        }
        loaded_orders.insert(hub_rel.clone(), order);
    }
    Ok(hub_rel)
}

/// Positional variant->hub frame pairing over two script frame-id orders:
/// zip, dedup, and drop variant frames that pair with conflicting hub frames
/// (those fall back to weaker contexts per sprite).
fn positional_pair_map(
    variant_order: &[u32],
    hub_order: &[u32],
) -> std::collections::BTreeMap<u32, u32> {
    let mut pairs: Vec<(u32, u32)> = variant_order
        .iter()
        .copied()
        .zip(hub_order.iter().copied())
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut pair_map = std::collections::BTreeMap::<u32, u32>::new();
    let mut conflicted = BTreeSet::<u32>::new();
    for (vid, hid) in pairs {
        match pair_map.get(&vid) {
            Some(&existing) if existing != hid => {
                conflicted.insert(vid);
            }
            Some(_) => {}
            None => {
                pair_map.insert(vid, hid);
            }
        }
    }
    for vid in &conflicted {
        pair_map.remove(vid);
    }
    pair_map
}

/// Sampled H(tile | above) * tile-count for one family member coded
/// standalone. `None` when an index doesn't fit the 12-bit proxy key.
fn family_base_standalone_proxy(holder: &FrameHolder, script_order: &[u32]) -> Option<f64> {
    let mut ids: Vec<u32> = script_order.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut joint = std::collections::HashMap::<u32, u32>::new();
    let mut ctx_totals = std::collections::HashMap::<u16, u32>::new();
    let mut sampled = 0u64;
    let mut full_tiles = 0u64;
    for &id in &ids {
        let sprite = holder.sprites().get(id as usize)?;
        if sprite.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(packed) = holder.packed_data(id) else {
            continue;
        };
        full_tiles += packed.len() as u64;
        if sampled >= FAMILY_PROXY_TILE_CAP {
            continue;
        }
        let cols = (sprite.width / 4) as usize;
        for (i, &x) in packed.iter().enumerate().skip(cols) {
            if x >= 4096 || packed[i - cols] >= 4096 {
                return None;
            }
            *joint
                .entry(((packed[i - cols] as u32) << 12) | x as u32)
                .or_default() += 1;
            *ctx_totals.entry(packed[i - cols]).or_default() += 1;
            sampled += 1;
        }
    }
    Some(family_proxy_bits(&joint, &ctx_totals, sampled, full_tiles))
}

/// Sampled H(member tile | candidate-base tile) * tile-count for coding
/// `member` against `candidate` (positional script pairing, mismatches
/// skipped like the real chunk builder).
fn family_base_pair_proxy(
    holder: &FrameHolder,
    candidate_order: &[u32],
    member_order: &[u32],
) -> Option<f64> {
    let mut pairs: Vec<(u32, u32)> = member_order
        .iter()
        .copied()
        .zip(candidate_order.iter().copied())
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut joint = std::collections::HashMap::<u32, u32>::new();
    let mut ctx_totals = std::collections::HashMap::<u16, u32>::new();
    let mut sampled = 0u64;
    let mut full_tiles = 0u64;
    for &(mid, cid) in &pairs {
        let (ms, cs) = (
            holder.sprites().get(mid as usize)?,
            holder.sprites().get(cid as usize)?,
        );
        if ms.dictionary_index == UNMAPPED_DICT || cs.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let (Some(mp), Some(cp)) = (holder.packed_data(mid), holder.packed_data(cid)) else {
            continue;
        };
        if (ms.width, ms.height) != (cs.width, cs.height) || mp.len() != cp.len() {
            continue;
        }
        full_tiles += mp.len() as u64;
        if sampled >= FAMILY_PROXY_TILE_CAP {
            continue;
        }
        for (&x, &b) in mp.iter().zip(cp.iter()) {
            if x >= 4096 || b >= 4096 {
                return None;
            }
            *joint.entry(((b as u32) << 12) | x as u32).or_default() += 1;
            *ctx_totals.entry(b).or_default() += 1;
            sampled += 1;
        }
    }
    Some(family_proxy_bits(&joint, &ctx_totals, sampled, full_tiles))
}

fn add_required_character_rhs_profiles_for_index(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    index: usize,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    add_character_rhs_profiles_for_index(required, profiles, index, in_path, true);
}

/// `required_on_disk = true` insists the RHS exists (mission-authored
/// characters must ship); `false` skips profiles whose RHS is absent from
/// this datadir entirely — the boot manifest indexes every CPF profile, but
/// a demo datadir only carries the files its missions can actually use.
fn add_character_rhs_profiles_for_index(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    index: usize,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
    required_on_disk: bool,
) {
    let Some(profile) = profiles.characters.get(index) else {
        return;
    };
    // Character profile indices identify physical RHS files. Do not group by
    // localized profile name: RobinHood and RobinTown can share one logical
    // name in legacy profile tables but the original PC constructor selects
    // exactly one physical variant from the level's forest flag.
    let rel = format!("Characters/{}.rhs", profile.filename);
    if required_on_disk || in_path(&rel).is_some() {
        add_required_rhs_rel(required, rel, &profile.profile_name);
    } else {
        tracing::warn!(
            "character profile '{}' ({}) has no RHS in this datadir; omitting from manifest index",
            profile.profile_name,
            profile.filename,
        );
    }
}

fn normalize_robin_profile_index(
    profiles: &ProfileManager,
    index: usize,
    forest_level: bool,
) -> Result<usize> {
    let profile = profiles
        .characters
        .get(index)
        .ok_or_else(|| anyhow!("character profile index {index} does not exist"))?;
    if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
        return Ok(index);
    }
    let wanted = if forest_level {
        "RobinHood"
    } else {
        "RobinTown"
    };
    profiles
        .characters
        .iter()
        .position(|candidate| candidate.filename == wanted)
        .ok_or_else(|| {
            anyhow!(
                "required {wanted} profile is absent while normalizing Robin for a {} mission",
                if forest_level { "forest" } else { "town" }
            )
        })
}

fn add_required_pc_profiles_for_pcs(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    pcs: &str,
    forest_level: bool,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    for profile_name in pcs.chars().filter_map(pc_code_profile_name) {
        let profile = profiles.characters.iter().find(|profile| {
            if profile.filename == "RobinHood" || profile.filename == "RobinTown" {
                profile.filename
                    == if forest_level {
                        "RobinHood"
                    } else {
                        "RobinTown"
                    }
            } else {
                profile.profile_name == profile_name
            }
        });
        if let Some(profile) = profile {
            let rel = format!("Characters/{}.rhs", profile.filename);
            if in_path(&rel).is_some() {
                add_required_rhs_rel(required, rel, &profile.profile_name);
            } else {
                tracing::warn!("demo PC profile '{}' has no shipped RHS", profile_name);
            }
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

fn add_character_action_rhs_profiles(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    actions: impl IntoIterator<Item = Action>,
) {
    for action in actions {
        let assets: &[(&str, &str)] = match action {
            Action::Bow => &[
                ("ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
                ("BONUS_Arrows", "BONUS Fleches"),
            ],
            Action::Stone => &[
                ("ACCESSORIES_Stone", "ACCESSOIRES Cailloux"),
                ("BONUS_Stones", "BONUS Cailloux"),
            ],
            Action::Apple => &[
                ("ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
                ("BONUS_Apples", "BONUS Pommes"),
            ],
            Action::Ale => &[
                ("ACCESSORIES_Ale", "ACCESSOIRES Ale"),
                ("BONUS_Ale", "BONUS Ale"),
            ],
            Action::Eat | Action::Guzzle => &[("BONUS_LegOfLamb", "BONUS Gigots")],
            Action::Heal => &[("BONUS_Plants", "BONUS Plantes")],
            Action::Net => &[
                ("ACCESSORIES_Net", "ACCESSOIRES Filet"),
                ("BONUS_Nets", "BONUS Filets"),
            ],
            Action::WaspNest => &[
                ("ACCESSORIES_Wasp", "ACCESSOIRES Guepes"),
                ("ACCESSORIES_WaspSting", "Guepe"),
                ("BONUS_WaspsNest", "BONUS Guepes"),
            ],
            Action::Purse => &[
                ("ACCESSORIES_MoneyBag", "ACCESSOIRES Bourse d'argent"),
                ("ACCESSORIES_Coin", "ACCESSOIRES Piece d'or"),
                ("BONUS_MoneyBag", "BONUS Bourses d'argent"),
            ],
            _ => &[],
        };
        for &(file, profile) in assets {
            add_required_rhs_rel(required, format!("Characters/{file}.rhs"), profile);
        }
    }
}

fn add_all_saved_world_object_rhs_profiles(
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
        ("BONUS_Arrows", "BONUS Fleches"),
        ("BONUS_Stones", "BONUS Cailloux"),
        ("BONUS_Nets", "BONUS Filets"),
        ("BONUS_WaspsNest", "BONUS Guepes"),
        ("BONUS_Apples", "BONUS Pommes"),
        ("BONUS_Ale", "BONUS Ale"),
        ("BONUS_LegOfLamb", "BONUS Gigots"),
        ("BONUS_Plants", "BONUS Plantes"),
        ("BONUS_MoneyBag", "BONUS Bourses d'argent"),
        ("BONUS_GoldBagsRansom", "BONUS Sac d'or rancon"),
        ("BONUS_Shield", "Shield"),
        ("BONUS_Parchment", "BONUS Parchemin"),
        ("BONUS_FourLeavedClover", "BONUS Trefle"),
        ("RELIC_Ampulla", "Huile"),
        ("RELIC_Spoon", "Cuillere"),
        ("RELIC_Crown", "Couronne"),
        ("RELIC_Stamp", "Sceau"),
        ("RELIC_Sceptre", "Sceptre"),
        ("RELIC_Book", "Registre"),
        ("RELIC_Sword", "Epee"),
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
