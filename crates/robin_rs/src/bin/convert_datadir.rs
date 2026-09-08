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

#[path = "convert_datadir/asset_resolution.rs"]
mod asset_resolution;
#[path = "convert_datadir/audio.rs"]
mod audio;
#[path = "convert_datadir/dependency_plan.rs"]
mod dependency_plan;
#[path = "convert_datadir/discovery.rs"]
mod discovery;
#[path = "convert_datadir/image_transform.rs"]
mod image_transform;
#[path = "convert_datadir/mission_planning.rs"]
mod mission_planning;
#[path = "convert_datadir/packaging.rs"]
mod packaging;
#[path = "convert_datadir/publication.rs"]
mod publication;
#[path = "convert_datadir/sprite_pipeline.rs"]
mod sprite_pipeline;
#[path = "convert_datadir/sprite_transform.rs"]
mod sprite_transform;

use asset_resolution::*;
use audio::*;
use dependency_plan::{DependencyPlan, DependencyRoot};
use discovery::*;
use image_transform::*;
use packaging::*;
use sprite_transform::*;

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

    /// Shipping: write the canonical browser Full-content package manifest.
    /// It binds every converted byte to the exact source Data/locale closure.
    #[arg(long)]
    web_content_manifest: bool,

    /// Shipping browser package's official edition. Required together with
    /// `--web-content-manifest`; the converter never guesses whether licensed
    /// source bytes are Demo or Full.
    #[arg(long, value_enum, requires = "web_content_manifest")]
    web_content_edition: Option<WebContentEditionArg>,

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

    /// Shipping: how to encode the RLE patch/ambient-animation sprite bucket
    /// (`Data/Animations/**` plus the ACCESSORIES_/BONUS_/RELIC_/TG_
    /// character files). `exact` keeps the byte-preserving packed RLE words
    /// (required for native/parity builds); `jxl-q70` ships them as lossy
    /// JXL per-animation atlases whose alpha channel carries the pixel
    /// class losslessly — transparent and shadow pixels stay bit-exact,
    /// only visible RGB is lossy (docs/COMPRESSION.md, 2026-08-30 RLE
    /// alpha-atlas section). WEB ONLY: it breaks framebuffer parity.
    #[arg(long, value_enum, default_value_t = RleSpriteFormat::Exact)]
    rle_sprite_format: RleSpriteFormat,
    /// Shipping: maximum VQ tiles per independent decoder job (whole grids).
    /// Zero preserves one adaptive stream per RHS for compression comparisons.
    #[arg(long, default_value_t = 0)]
    vq_group_tiles: usize,
    /// Shipping: independent JXL atlases per decoder job; zero keeps each RHS together.
    #[arg(long, default_value_t = 0)]
    rle_group_blobs: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum RleSpriteFormat {
    /// Keep exact packed RLE words (byte-preserving).
    Exact,
    /// Lossy JXL atlases at quality 70 + lossless class masks (web recipe).
    JxlQ70,
    /// Lossy JXL atlases at quality 80 + lossless class masks.
    JxlQ80,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum WebContentEditionArg {
    Demo,
    Full,
}

impl From<WebContentEditionArg> for robin_rs::multiplayer::content_identity::WebContentEdition {
    fn from(value: WebContentEditionArg) -> Self {
        match value {
            WebContentEditionArg::Demo => Self::Demo,
            WebContentEditionArg::Full => Self::Full,
        }
    }
}

impl RleSpriteFormat {
    fn jxl_quality(self) -> Option<u8> {
        match self {
            Self::Exact => None,
            Self::JxlQ70 => Some(70),
            Self::JxlQ80 => Some(80),
        }
    }
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
        OutFormat::Hackable if args.web_content_manifest => {
            bail!("--web-content-manifest is supported only with --format shipping")
        }
        OutFormat::Hackable => Converter::new(data_in, data_out).run(),
        OutFormat::Shipping => {
            let web_content_edition = match (args.web_content_manifest, args.web_content_edition) {
                (true, Some(edition)) => {
                    Some(validate_web_content_edition(&data_in, edition.into())?)
                }
                (true, None) => {
                    bail!("--web-content-manifest requires --web-content-edition demo|full")
                }
                (false, None) => None,
                (false, Some(_)) => unreachable!("clap enforces --web-content-manifest"),
            };
            let native_content_sha256 = args
                .web_content_manifest
                .then(|| {
                    robin_rs::multiplayer::content_identity::source_content_identity(&data_in)
                        .map_err(anyhow::Error::msg)
                })
                .transpose()?;
            convert_shipping(
                data_in,
                &data_out,
                ShippingOpts {
                    map_format: args.map_format,
                    interface_image_format: args.interface_image_format,
                    audio_format: args.audio_format,
                    zstd_window_log: args.zstd_window_log,
                    resume: args.resume,
                    rank_dictionaries: args.rank_dictionaries,
                    rle_sprite_format: args.rle_sprite_format,
                    vq_group_tiles: args.vq_group_tiles,
                    rle_group_blobs: args.rle_group_blobs,
                },
            )?;
            if let Some(identity) = native_content_sha256 {
                write_web_content_manifest(
                    &data_out,
                    web_content_edition.expect("manifest edition was validated"),
                    identity,
                )?;
            }
            Ok(())
        }
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
    rle_sprite_format: RleSpriteFormat,
    vq_group_tiles: usize,
    rle_group_blobs: usize,
}

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
            // packed 16-bit picture on disk — the same compressed
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
            // font parser that we haven't implemented.
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
            sprite_script::SpriteScriptor::load_all_profiles_legacy(&src.to_string_lossy())
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
        AudioFormat, AudioKind, InterfaceImageFormat, add_character_action_rhs_profiles,
        animation_rhs_paths, animation_rhs_rel_existing, detect_locale_data_dirs,
        detect_official_web_content_edition, exclamation_dat_filename, find_data_dir,
        insert_shipping_audio, insert_standalone_audio, is_common_audio_member, lcid_to_iso,
        level_asset_rel_existing, normalize_robin_profile_index, positional_pair_map,
        prepare_shipping_payload, sxt_is_sres, transcode_audio_to_opus, transcode_sxt_drop_bzip,
        validate_web_content_edition, walk_and_bundle_locale, write_shipping_dependency,
    };
    use robin_assets::picture::{Picture, PixelFormat, SixteenPacking};
    use robin_assets::shipping_datadir::{ShippingAudioAsset, ShippingLocale, ShippingMission};
    use robin_engine::profiles::{Action, CharacterProfile, ProfileManager};
    use robin_rs::multiplayer::content_identity::WebContentEdition;
    use std::fs;
    use std::path::Path;

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
    fn common_audio_excludes_menu_exclamations_and_mission_dialogue() {
        let dialogue = std::collections::BTreeSet::from(["sounds/dialog/line.wav".into()]);
        assert!(is_common_audio_member("arrow_hit.wav", &dialogue));
        assert!(!is_common_audio_member("snd_001.wav", &dialogue));
        assert!(!is_common_audio_member("menu/click.wav", &dialogue));
        assert!(!is_common_audio_member(
            "exclamations/robin/alert.wav",
            &dialogue
        ));
        assert!(!is_common_audio_member("dialog/line.wav", &dialogue));
    }

    #[test]
    fn opus_payload_retains_exact_catalog_membership_without_encoded_bytes() {
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
        let source = temp.path().join("arrow.wav");
        std::fs::write(&source, wav).unwrap();
        let mut payload = ShippingMission::default();
        let mut catalog = std::collections::BTreeMap::from([(
            "sounds/arrow.opus".into(),
            ShippingAudioAsset {
                file: "audio/assets/existing.opus".into(),
                encoded_size: 42,
                duration_ms: 100,
                bundle_offset: None,
            },
        )]);

        insert_shipping_audio(
            &mut payload,
            &mut catalog,
            &temp.path().join("audio/assets"),
            "common",
            "Sounds/Arrow.wav",
            &source,
            AudioKind::Effect,
            AudioFormat::Opus,
        )
        .unwrap();

        assert!(payload.raw.is_empty());
        assert_eq!(payload.audio_durations_ms["sounds/arrow.opus"], 100);
    }

    #[test]
    fn opus_membership_only_dependency_is_written_and_decodes() {
        let temp = tempfile::tempdir().unwrap();
        let mut payload = ShippingMission::default();
        payload
            .audio_durations_ms
            .insert("sounds/arrow.opus".into(), 100);

        let relative =
            write_shipping_dependency(temp.path(), "metadata-only-audio", &payload, 30, false)
                .unwrap()
                .expect("Opus membership metadata is a real dependency");
        let filename = std::path::Path::new(&relative)
            .file_name()
            .expect("dependency path has a file name");
        let compressed = std::fs::read(temp.path().join(filename)).unwrap();
        let decoded = robin_assets::shipping_datadir::decode_mission_compressed(&compressed)
            .expect("decode metadata-only dependency");

        assert!(decoded.raw.is_empty());
        assert_eq!(decoded.audio_durations_ms["sounds/arrow.opus"], 100);
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

    #[test]
    fn locale_detection_preserves_every_installed_pack() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("Data");
        fs::create_dir(&base).unwrap();
        for lcid in ["1033", "1031", "1036", "2047"] {
            fs::create_dir_all(temp.path().join(lcid).join("Data")).unwrap();
        }

        let detected = detect_locale_data_dirs(&base);
        let identities = detected
            .iter()
            .map(|source| (source.lcid, source.iso))
            .collect::<Vec<_>>();
        assert_eq!(identities[0], ("1033", "en-US"));
        assert!(identities.contains(&("1031", "de-DE")));
        assert!(identities.contains(&("1036", "fr-FR")));
        assert!(identities.contains(&("2047", "und")));
        assert_eq!(identities.len(), 4);
        assert_eq!(lcid_to_iso("2047"), "und");
    }

    #[test]
    fn sxt_dispatches_standalone_sixteen_picture_by_content() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Start.sxt");
        let pixels = vec![0x00, 0x00, 0x1f, 0x00, 0xe0, 0x07, 0xff, 0xff];
        let picture = Picture {
            width: 2,
            height: 2,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: pixels.clone(),
            palette: None,
        };
        fs::write(
            &path,
            picture
                .write_sixteen_to_bytes(SixteenPacking::Bzip)
                .unwrap(),
        )
        .unwrap();

        assert!(!sxt_is_sres(&path).unwrap());
        let converted = transcode_sxt_drop_bzip(&path).unwrap();
        assert_eq!(u32::from_le_bytes(converted[4..8].try_into().unwrap()), 0);
        let decoded = Picture::load_sixteen_from_bytes(&converted).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(decoded.data, pixels);
    }

    #[test]
    fn sxt_recognizes_sres_resource_container_magic() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("strings.sxt");
        fs::write(&path, b"SRES\0\x01\0\0\0\0\0\0").unwrap();

        assert!(sxt_is_sres(&path).unwrap());
    }

    #[test]
    fn authentic_demo_start_sxt_is_a_sixteen_picture_when_available() {
        // The licensed corpus is intentionally not checked into the repo.
        // Exercise it when the operator has mounted the authentic Demo loose
        // root, while keeping this regression runnable in public checkouts.
        let path = Path::new("datadirs/demo_loose/1033/data/Interface/Start.sxt");
        if !path.is_file() {
            return;
        }

        assert!(!sxt_is_sres(path).unwrap());
        let converted = transcode_sxt_drop_bzip(path).unwrap();
        let decoded = Picture::load_sixteen_from_bytes(&converted).unwrap();
        assert_eq!((decoded.width, decoded.height), (1024, 768));
        assert_eq!(decoded.data.len(), 1024 * 768 * 2);
        assert_eq!(u32::from_le_bytes(converted[4..8].try_into().unwrap()), 0);
    }

    #[test]
    fn locale_start_sxt_is_bundled_as_a_standalone_picture() {
        let temp = tempfile::tempdir().unwrap();
        let interface = temp.path().join("Interface");
        fs::create_dir(&interface).unwrap();
        let picture = Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: vec![0x34, 0x12, 0x78, 0x56],
            palette: None,
        };
        fs::write(
            interface.join("Start.sxt"),
            picture
                .write_sixteen_to_bytes(SixteenPacking::Bzip)
                .unwrap(),
        )
        .unwrap();

        let mut locale = ShippingLocale::default();
        walk_and_bundle_locale(
            &mut locale,
            temp.path(),
            temp.path(),
            InterfaceImageFormat::Raw,
        )
        .unwrap();

        assert!(locale.res_files.is_empty());
        let bundled = &locale.raw["interface/start.sxt"];
        let decoded = Picture::load_sixteen_from_bytes(bundled).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.data, picture.data);
        assert_eq!(u32::from_le_bytes(bundled[4..8].try_into().unwrap()), 0);
    }

    #[test]
    fn mixed_official_source_markers_are_rejected_as_ambiguous() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("Data");
        fs::create_dir_all(data.join("Levels")).unwrap();
        fs::write(data.join("Levels/Dem_Lei_MP.rhm"), b"demo marker").unwrap();
        fs::write(data.join("Levels/Sherwood.rhm"), b"full marker").unwrap();

        let error = detect_official_web_content_edition(&data).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("ambiguous"));
        assert!(message.contains("Dem_Lei_MP.rhm"));
        assert!(message.contains("Sherwood.rhm"));
    }

    #[test]
    fn authentic_demo_and_full_loose_roots_have_exact_typed_editions_when_available() {
        // Licensed source bytes stay external to the repository. Exercise both
        // mounted operator corpora when present without embedding fixtures.
        let cases = [
            ("datadirs/demo_loose", WebContentEdition::Demo),
            ("datadirs/full_loose", WebContentEdition::Full),
        ];
        for (root, expected) in cases {
            let root = Path::new(root);
            if !root.is_dir() {
                continue;
            }
            let data = find_data_dir(root).unwrap();
            assert_eq!(
                detect_official_web_content_edition(&data).unwrap(),
                expected,
                "wrong edition for {}",
                root.display()
            );
            assert_eq!(
                validate_web_content_edition(&data, expected).unwrap(),
                expected
            );
            let opposite = match expected {
                WebContentEdition::Demo => WebContentEdition::Full,
                WebContentEdition::Full => WebContentEdition::Demo,
            };
            assert!(validate_web_content_edition(&data, opposite).is_err());
        }
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
    let mut mgr = ResourceManager::legacy_tool();
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

/// `.pak` files hold a handful of sequential packed 16-bit images.
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

/// Decode a packed 16-bit (`.map` / `.min`) image and re-encode it
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
    RhsData, RleJxlPlacement, ShippingAudioAsset, ShippingDatadir, ShippingLocale, ShippingMission,
    ShippingMissionRef, ShippingSprite, ShippingSpriteBank, SpriteRleJxlChunk, SpriteVqChunk,
    canonical_shipping_asset_key,
};
use robin_engine::level_data::LoadedLevel;

#[derive(Default)]
struct ShippingMissionBuild {
    payload: ShippingMission,
    required_rhs_profiles: std::collections::BTreeMap<String, BTreeSet<String>>,
    required_exclamation_ids: BTreeSet<u32>,
    music_names: BTreeSet<String>,
    dialogue_samples: BTreeSet<String>,
    sound_wave_ids: BTreeSet<u32>,
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

fn convert_shipping(data_in: PathBuf, data_out: &Path, opts: ShippingOpts) -> Result<()> {
    let mut dd = ShippingDatadir::default();
    let mut beggar_ids: BTreeSet<u32> = BTreeSet::new();
    let audio_assets_dir = data_out.join("audio/assets");
    fs::create_dir_all(&audio_assets_dir)?;

    let locale_dirs = detect_locale_data_dirs(&data_in);
    for src in &locale_dirs {
        tracing::info!("Locale data dir [{}]: {}", src.iso, src.data_dir.display());
        let mut aliases = BTreeSet::from([src.lcid.to_owned(), src.iso.to_owned()]);
        if src.iso == "und" {
            aliases.insert("neutral".to_owned());
        }
        let locale = ShippingLocale {
            source_lcid: Some(src.lcid.to_owned()),
            aliases,
            ..ShippingLocale::default()
        };
        if dd.locales.insert(src.iso.to_owned(), locale).is_some() {
            bail!(
                "multiple locale directories resolve to canonical locale {}",
                src.iso
            );
        }
    }

    // Top-level fields retain the v4 default-resolution behavior for existing
    // consumers: base Data first, English fallback, then the remaining locale
    // dirs. Explicit per-locale maps below never use this fallback closure.
    let in_path = |rel: &str| -> Option<PathBuf> {
        if let Some(resolved) = resolve_data_file(&data_in, rel) {
            return Some(resolved);
        }
        for alt in &locale_dirs {
            if let Some(resolved) = resolve_data_file(&alt.data_dir, rel) {
                return Some(resolved);
            }
        }
        None
    };

    // ── Fixed boot roots ───────────────────────────────────────────────
    // Boot-time resource roots plus the expression/actor text
    // table and loading-screen bundle.
    for rel in [
        "Interface/DEFAULT.RES",
        "Interface/Start.sxt",
        "Text/actors.res",
        "Text/Level.res",
        "Sounds/Exclamations/actors.res",
    ] {
        if let Some(p) = in_path(rel) {
            // `.sxt` is an extension used by more than one legacy wire
            // format. Some releases store Start.sxt as an SRES text table,
            // while the authentic demo stores a standalone 1024x768
            // packed 16-bit loading image. Only attach actual SRES
            // containers; the standalone picture is validated/transcoded by
            // walk_and_bundle_small below and retained in `raw`.
            if rel.to_ascii_lowercase().ends_with(".sxt") && !sxt_is_sres(&p)? {
                continue;
            }
            let mut mgr = ResourceManager::legacy_tool();
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
    for source in &locale_dirs {
        let locale = dd
            .locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized");
        for rel in [
            "Interface/DEFAULT.RES",
            "Interface/Start.sxt",
            "Text/actors.res",
            "Text/Level.res",
            "Sounds/Exclamations/actors.res",
        ] {
            let Some(path) = resolve_data_file(&source.data_dir, rel) else {
                continue;
            };
            // See the matching default-locale loop above: a localized SXT
            // may be either an SRES archive or a standalone Sixteen image.
            if rel.to_ascii_lowercase().ends_with(".sxt") && !sxt_is_sres(&path)? {
                continue;
            }
            let mut mgr = ResourceManager::legacy_tool();
            mgr.attach_resource_file(&path.to_string_lossy())?;
            if is_interface_path(rel)
                && let Some(quality) = opts.interface_image_format.jxl_quality()
            {
                let encoded = mgr.encode_pictures_for_shipping(|picture| {
                    Ok(EncodedPicture::jxl_rgba565_keyed(
                        transcode_picture_to_jxl_rgba_keyed(picture, quality)?,
                    ))
                })?;
                tracing::info!(
                    locale = source.iso,
                    "interface res {rel}: encoded {encoded} pictures as JXL {}",
                    jxl_quality_label(quality)
                );
            }
            locale
                .res_files
                .insert(canonical_shipping_asset_key(rel), mgr);
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
    dd.raw.extend(boot_audio.payload.raw);
    dd.audio_durations_ms
        .extend(boot_audio.payload.audio_durations_ms);

    if opts.interface_image_format != InterfaceImageFormat::Raw {
        for source in &locale_dirs {
            let Some(path) = resolve_data_file(&source.data_dir, "Interface/Loading.pak") else {
                continue;
            };
            let pictures = read_pak_pictures(&path)?;
            let encoded = encode_interface_pak_pictures(&pictures, opts.interface_image_format)?;
            dd.locales
                .get_mut(source.iso)
                .expect("detected shipping locale was initialized")
                .pak_files
                .insert("interface/loading.pak".into(), encoded);
        }
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

    let mut mission_builds =
        mission_planning::plan_missions(&mut dd, &cpf, &locale_dirs, &beggar_ids, &in_path)?;

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
        rle_jxl_chunks: Vec::new(),
    });
    let mut dependency_plan = DependencyPlan::default();
    for (mission, build) in &mission_builds {
        dependency_plan.include(
            DependencyRoot::Mission(mission.clone()),
            &build.required_rhs_profiles,
        );
    }
    for (character, required) in &character_rhs_requirements {
        dependency_plan.include(DependencyRoot::Character(*character), required);
    }
    for (mission, build) in &mission_builds {
        let mut planned = dependency_plan::PlannedMission::default();
        planned.sources.insert(
            format!("Levels/{}.rhp", build.proto_filename),
            "mission proto level".into(),
        );
        planned
            .sources
            .insert(format!("Levels/{mission}.rhm"), "mission world".into());
        for map in &build.map_names {
            planned
                .sources
                .insert(map.clone(), "terrain map and minimap".into());
        }
        for music in &build.music_names {
            planned
                .sources
                .insert(format!("Musics/{music}"), "mission music".into());
        }
        for dialogue in &build.dialogue_samples {
            planned
                .sources
                .insert(dialogue.clone(), "localized mission dialogue".into());
        }
        for id in &build.sound_wave_ids {
            planned
                .sources
                .insert(format!("Sounds/snd_{id:03}"), "mission sound source".into());
        }
        for id in &build.required_exclamation_ids {
            planned.sources.insert(
                format!("exclamation:{id:08x}"),
                "actor voice profile".into(),
            );
        }
        dependency_plan.missions.insert(mission.clone(), planned);
    }
    dependency_plan.include(DependencyRoot::SavedWorld, &saved_world_rhs_requirements);
    // Preserve the planned roots even if a later codec or source read fails.
    write_json_pretty(&data_out.join("conversion-plan.json"), &dependency_plan)?;
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

    let (rhs_payloads, rhs_base_dep) = sprite_pipeline::transform_rhs(
        &data_in,
        &holder,
        dict_remaps.as_deref(),
        &opts,
        &compression_pool,
        &dependency_plan,
        &in_path,
    )?;

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
    for source in &locale_dirs {
        let Some(path) = resolve_data_file(&source.data_dir, "Configuration/profile.cpf") else {
            continue;
        };
        let mut file = SbFile::open(&path.to_string_lossy(), SB_FILE_READ)
            .map_err(|error| anyhow!("open locale {} cpf: {error}", source.iso))?;
        let mut profiles = ProfileManager::new();
        profiles
            .load_all_legacy_cpf(&mut file)
            .map_err(|error| anyhow!("parse locale {} cpf: {error}", source.iso))?;
        if let Some(level_dir) = resolve_case_insensitive(&data_in.join("Levels"))
            .filter(|path| path.is_dir())
            .map(|path| path.to_string_lossy().into_owned())
        {
            profiles.import_beam_mes(&level_dir);
        }
        dd.locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized")
            .profiles = Some(profiles);
    }

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
        "res", "sxt", "pak", "red", // Small shared resource bundles
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
    // The v5 locale dimension is complete rather than boot-file-only: voice,
    // dialogue, and cinematics are language assets too. Keeping each overlay
    // self-contained lets the same in-memory VFS bundle work on desktop,
    // browser, and Android. The top-level compatibility maps above retain the
    // historical compact/default-language view for old consumers.
    for source in &locale_dirs {
        let locale = dd
            .locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized");
        walk_and_bundle_locale(
            locale,
            &source.data_dir,
            &source.data_dir,
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
    for (rel, planned) in &mut dependency_plan.rhs {
        planned.destination_payloads = rhs_chunk_files(rel)?;
        planned.grouping = Some(match rhs_base_dep.get(rel).map(Vec::len).unwrap_or(0) {
            0 => "standalone".to_owned(),
            count => format!("family variant with {count} shared hub(s)"),
        });
    }

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
    // Dialogue and `snd_NNN` source waves receive exact per-mission metadata
    // below. Keeping either in this shared payload would make active warmup
    // falsely treat every campaign mission's speech/ambience as required.
    let mission_dialogue_keys: BTreeSet<String> = mission_builds
        .values()
        .flat_map(|build| build.dialogue_samples.iter())
        .map(|path| robin_util::asset_fs::bundle_key(Path::new(path)))
        .collect();
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
            if !is_common_audio_member(&relative, &mission_dialogue_keys) {
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
    let mut actors_res = ResourceManager::legacy_tool();
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
                    sound_wave_ids,
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
                    sound_wave_ids,
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
            sound_wave_ids,
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
        let mut source_audio = ShippingMission::default();
        for id in sound_wave_ids {
            let resolved = ["wav", "ogg"].into_iter().find_map(|extension| {
                let relative = format!("Sounds/snd_{id:03}.{extension}");
                in_path(&relative).map(|path| (relative, path))
            });
            let Some((relative, path)) = resolved else {
                tracing::warn!(
                    mission = mission_name,
                    id,
                    "mission sound source has no sample"
                );
                continue;
            };
            insert_shipping_audio(
                &mut source_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                &format!("ambience-{}", shipping_file_stem(&mission_name)),
                &relative,
                &path,
                AudioKind::Effect,
                opts.audio_format,
            )?;
        }
        if let Some(file) = write_shipping_dependency(
            &audio_dir,
            "mission-ambience",
            &source_audio,
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
    for (mission, planned) in &mut dependency_plan.missions {
        planned.destination_payloads = dd
            .missions
            .get(mission)
            .ok_or_else(|| anyhow!("planned mission {mission} has no packaged payload"))?
            .files
            .clone();
    }
    // Diagnostic plan is separate from the encoded datadir and web manifest.
    publication::publish_shipping(&dd, data_out, &opts)?;
    dependency_plan.completed = true;
    write_json_pretty(&data_out.join("conversion-plan.json"), &dependency_plan)
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

fn exclamation_dat_filename(exclamation_id: u32) -> String {
    let suffix: String = exclamation_id
        .to_le_bytes()
        .into_iter()
        .filter(|byte| *byte != 0)
        .map(char::from)
        .collect();
    format!("actor{suffix}.dat")
}

/// Recursively bundle small files under their canonical logical keys.
/// Drop inner picture bzip2 wrappers: outer shipping compression replaces it.
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
            "sxt" => transcode_sxt_drop_bzip(&path)
                .with_context(|| format!("transcode sxt {}: keeping raw bytes", path.display()))?,
            "bfn" => transcode_bfn_drop_bzip(&path)
                .with_context(|| format!("transcode bfn {}", path.display()))?,
            _ => fs::read(&path)
                .with_context(|| format!("walk_and_bundle_small: read {}", path.display()))?,
        };
        dd.raw.insert(rel, bytes);
    }
    Ok(())
}

/// Recursively preserve one complete locale overlay. Unlike the top-level
/// boot bundle this intentionally includes large speech/cinematic assets: a
/// browser or Android build cannot reach loose host files after switching.
fn walk_and_bundle_locale(
    locale: &mut ShippingLocale,
    root: &Path,
    src: &Path,
    interface_image_format: InterfaceImageFormat,
) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_and_bundle_locale(locale, root, &path, interface_image_format)?;
            continue;
        }
        if !path.is_file() {
            tracing::warn!("skipping non-file locale asset {}", path.display());
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .with_context(|| {
                format!(
                    "locale asset {} is outside root {}",
                    path.display(),
                    root.display()
                )
            })?
            .to_string_lossy();
        let key = canonical_shipping_asset_key(&rel);
        if locale.raw.contains_key(&key) {
            bail!("duplicate locale asset key {key}");
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);

        let is_resource_container = extension.as_deref() == Some("res")
            || (extension.as_deref() == Some("sxt") && sxt_is_sres(&path)?);
        if is_resource_container && !locale.res_files.contains_key(&key) {
            let mut resources = ResourceManager::legacy_tool();
            resources
                .attach_resource_file(&path.to_string_lossy())
                .with_context(|| format!("parse locale resource {}", path.display()))?;
            if is_interface_path(&key)
                && let Some(quality) = interface_image_format.jxl_quality()
            {
                resources.encode_pictures_for_shipping(|picture| {
                    Ok(EncodedPicture::jxl_rgba565_keyed(
                        transcode_picture_to_jxl_rgba_keyed(picture, quality)?,
                    ))
                })?;
            }
            locale.res_files.insert(key.clone(), resources);
        }
        if extension.as_deref() == Some("red") {
            let filename = key.rsplit('/').next().unwrap_or(&key).to_owned();
            if !locale.red_files.contains_key(&filename) {
                locale.red_files.insert(
                    filename,
                    res_descr::load(&path.to_string_lossy())
                        .with_context(|| format!("parse locale descriptor {}", path.display()))?,
                );
            }
        }

        if interface_image_format != InterfaceImageFormat::Raw
            && is_interface_path(&key)
            && extension.as_deref() == Some("pak")
        {
            let pictures = read_pak_pictures(&path)?;
            locale.pak_files.insert(
                key,
                encode_interface_pak_pictures(&pictures, interface_image_format)?,
            );
            continue;
        }

        let bytes = match extension.as_deref() {
            Some("pak") => transcode_pak_drop_bzip(&path)
                .with_context(|| format!("transcode locale pak {}", path.display()))?,
            Some("res") => transcode_res_drop_bzip(&path)
                .with_context(|| format!("transcode locale res {}", path.display()))?,
            Some("sxt") => transcode_sxt_drop_bzip(&path)
                .with_context(|| format!("transcode locale sxt {}", path.display()))?,
            Some("bfn") => transcode_bfn_drop_bzip(&path)
                .with_context(|| format!("transcode locale bfn {}", path.display()))?,
            _ => {
                fs::read(&path).with_context(|| format!("read locale asset {}", path.display()))?
            }
        };
        locale.raw.insert(key, bytes);
    }
    Ok(())
}

/// `.pak` files are a back-to-back sequence of packed-picture blobs —
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

/// `.min` / `.map` bitmaps: a single packed 16-bit picture. Decode the
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

/// Return whether an `.sxt` uses the SRES resource-container wire format.
///
/// The extension is not a format discriminator in the original data. Retail
/// variants may use it for an SRES string table, while the authentic demo's
/// `Interface/Start.sxt` is a standalone packed 16-bit picture. Keep the sniff
/// deliberately strict: a non-SRES SXT is passed to the checked Sixteen
/// decoder below, so an unknown or corrupt file fails closed rather than
/// being copied or silently omitted.
fn sxt_is_sres(path: &Path) -> Result<bool> {
    use std::io::Read as _;

    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .with_context(|| format!("read SXT header {}", path.display()))?;
    Ok(&magic == b"SRES")
}

/// Transcode either legacy format carried by the `.sxt` extension.
fn transcode_sxt_drop_bzip(path: &Path) -> Result<Vec<u8>> {
    if sxt_is_sres(path)? {
        transcode_res_drop_bzip(path)
    } else {
        transcode_sixteen_drop_bzip(path)
    }
}

/// `.bfn` native font files: a fixed header + `char_number` character
/// records + two back-to-back packed 16-bit pictures (glyph atlas
/// plus alpha mask). The picture payloads ship `SixteenPacking::Bzip`
/// on the original retail discs, so we decode them now and re-emit the
/// whole file with `SixteenPacking::None`. Matches the
/// native-font loading format — see `crate::native_font` for the
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

    // Decode both packed-picture payloads via the existing
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

/// Return the number of bytes a packed 16-bit picture occupies at
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
    let mut rm = ResourceManager::legacy_tool();
    rm.attach_resource_file(&path.to_string_lossy())?;
    rm.write_to_res_bytes(SixteenPacking::None)
}
