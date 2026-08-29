//! Lossy-JXL probe for large VQ character sprites (research harness).
//!
//! Question under test (docs/COMPRESSION.md, 2026-08-29 section): the VQ
//! sprite data is already lossy (inherited from the original game's
//! vector quantisation), so would *lossy* JXL on the DECODED RGB565 pixels
//! beat the shipping `sprite_codec` context-model blobs for sprites larger
//! than ~20x20 px? Earlier investigations only closed the door on lossless
//! image codecs and on lossy JXL over *atlas/verbatim* representations.
//!
//! Method, per RHS character:
//!   1. Collect every bank sprite referenced by the RHS scripts; keep the
//!      well-formed VQ ones, and select those with width>=20 && height>=20.
//!   2. Current-codec comparators (both via `sprite_codec`, reproducing the
//!      converter's standalone chunk path with derived self-references):
//!      - `blob_all`: every VQ grid of the character in one blob (matches
//!        the shipping chunk; share of the selected sprites is prorated by
//!        tile count), and
//!      - `blob_sel`: ONLY the selected sprites re-encoded — the exact
//!        byte comparator for the JXL side.
//!   3. JXL side: decode each selected sprite to RGB565 (Day dictionary),
//!      export an RGBA PNG, and encode with `cjxl` at -q 90, -q 80 and
//!      lossless -d 0. Key handling (documented choice): the transparent
//!      key 0x07C0 AND the shadow key 0x001F are both excluded from the
//!      JXL image (alpha 0, RGB free for the encoder to discard); a 2-bit
//!      per-pixel class map (transparent/shadow/opaque) is packed and
//!      zstd-compressed per character and counted toward the JXL totals.
//!      Reconstruction takes keys from the class map — the JXL alpha
//!      channel is only an encoder hint — so keyed transparency and hard
//!      shadows are exact by construction and the quality question reduces
//!      to RGB error on opaque pixels.
//!   4. Atlas variant: the animation (profile, action) with the most
//!      selected frames is packed into one grid atlas and encoded once,
//!      to measure how much of the per-image JXL header tax an atlas
//!      recovers.
//!   5. Quality/decode-side: every lossy JXL is decoded back with `jxl-rs`
//!      (the runtime's decoder), timed, requantised to RGB565, and scored
//!      (PSNR over opaque pixels, % of opaque pixels exact in RGB565,
//!      would-be key collisions). Worst sprites are dumped side-by-side
//!      as PNGs under the output directory.
//!
//! Follow-up modes (same doc section, RLE/patch bucket and loading art —
//! content where the economics differ because there is no VQ codec side):
//!
//! - `--rle`: the RLE/patch bucket (Characters/ACCESSORIES_*|BONUS_*|
//!   RELIC_*|TG_* plus Data/Animations/**/*.rhs, first-claim dedup —
//!   the exact bucket of the "RLE bucket context modeling" ledger entry).
//!   Comparators are zstd-19 and xz -9e over the ledger's corpus-blob
//!   layout (`w,h,dict,len,packed` per sprite); the JXL side gets the same
//!   2-bit class-map treatment as VQ mode with one extra class for
//!   in-run literals that carry the transparent-key VALUE (so the exact
//!   RLE stream stays reconstructible: run extents come from the map,
//!   literal values from the image). Per-sprite q90/q80/q70/d0, plus
//!   per-animation (rhs,profile,action) grid atlases and — where cjxl
//!   accepts APNG input — animated-JXL encodes of the same groups.
//! - `--pak`: the loading-art `.pak` pictures (`Data/Interface/Loading.pak`,
//!   `2047/Data/Interface/Slideshow_in.pak` by default, `--pak-file <rel>`
//!   to override). RGB565 `SBPictureSixteen` frames, keyed RGBA export
//!   exactly like the converter's interface path (cjxl e9), q90/q80/q70/d0
//!   vs the raw and zstd-max lossless baselines.
//!
//! Usage:
//!
//! ```text
//! cargo build --release --example jxl_sprite_probe
//! cargo run --release --example jxl_sprite_probe -- \
//!     --data-dir datadirs/fullgame_linux \
//!     --out tmp/jxl_sprite_probe \
//!     --cjxl /path/to/cjxl \
//!     [--rhs "Characters/Knight01.rhs"]... [--min-dim 20] [--limit 0] \
//!     [--rle] [--pak] [--pak-file <rel>]...
//! ```

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(target_arch = "wasm32"))]
mod probe {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use anyhow::{Context, Result, anyhow, bail};
    use rayon::prelude::*;
    use robin_assets::frame_holder::{
        FrameHolder, SHADOW_KEY, TRANSPARENT_COLOR_16, UNMAPPED_DICT,
    };
    use robin_assets::picture::{Picture, PixelFormat};
    use robin_assets::shipping_datadir::derive_chunk_self_refs;
    use robin_assets::sprite_codec::{self, SpriteGrid};
    use robin_engine::sbfile::{SB_FILE_READ, SbFile};
    use robin_engine::sprite_script::{SpriteInfo, SpriteScriptor};
    use robin_engine::sprite_variant::SpriteVariant;

    const DEFAULT_RHS: &[&str] = &[
        "Characters/Knight01.rhs",
        "Characters/RobinTown.rhs",
        "Characters/Guard A00.rhs",
        "Animations/Day/chariot01.rhs",
        "Animations/Day/sherwood.rhs",
    ];

    struct Args {
        data_dir: String,
        out: PathBuf,
        rhs: Vec<String>,
        min_dim: u16,
        cjxl: String,
        limit: usize,
        keep_pngs: bool,
        rle: bool,
        pak: bool,
        pak_files: Vec<String>,
    }

    fn parse_args() -> Result<Args> {
        let mut args = Args {
            data_dir: "datadirs/fullgame_linux".to_owned(),
            out: PathBuf::from("tmp/jxl_sprite_probe"),
            rhs: Vec::new(),
            min_dim: 20,
            cjxl: "cjxl".to_owned(),
            limit: 0,
            keep_pngs: false,
            rle: false,
            pak: false,
            pak_files: Vec::new(),
        };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().ok_or_else(|| anyhow!("missing value for {name}"));
            match a.as_str() {
                "--data-dir" => args.data_dir = val("--data-dir")?,
                "--out" => args.out = PathBuf::from(val("--out")?),
                "--rhs" => args.rhs.push(val("--rhs")?),
                "--min-dim" => args.min_dim = val("--min-dim")?.parse()?,
                "--cjxl" => args.cjxl = val("--cjxl")?,
                "--limit" => args.limit = val("--limit")?.parse()?,
                "--keep-pngs" => args.keep_pngs = true,
                "--rle" => args.rle = true,
                "--pak" => args.pak = true,
                "--pak-file" => {
                    args.pak = true;
                    args.pak_files.push(val("--pak-file")?);
                }
                other => bail!("unknown argument {other}"),
            }
        }
        if args.rhs.is_empty() {
            args.rhs = DEFAULT_RHS.iter().map(|s| (*s).to_owned()).collect();
        }
        Ok(args)
    }

    /// One selected sprite, fully expanded. In VQ (character) mode `grid`
    /// holds the tile-index grid; in `--rle` mode it holds the raw packed
    /// RLE words (the ledger corpus payload for that sprite).
    struct Sel {
        id: u32,
        width: u16,
        height: u16,
        grid: Vec<u16>,
        /// Decoded RGB565 pixels, `width x height`.
        pixels: Vec<u16>,
        /// Per-pixel class (`CL_*`), same length as `pixels`.
        classes: Vec<u8>,
    }

    /// 2-bit pixel classes. VQ sprites only use 0..=2; RLE mode adds class 3
    /// for the rare in-run literal that carries the transparent-key VALUE
    /// (the position is inside the run, so it is not background — with the
    /// class map, run extents AND key literals reconstruct exactly).
    const CL_TRANS: u8 = 0; // transparent / outside any RLE run
    const CL_SHADOW: u8 = 1; // shadow-key pixel
    const CL_OPAQUE: u8 = 2; // real color
    const CL_KEYLIT: u8 = 3; // RLE literal with the transparent-key value

    fn classify(px: u16) -> u8 {
        match px {
            TRANSPARENT_COLOR_16 => CL_TRANS,
            SHADOW_KEY => CL_SHADOW,
            _ => CL_OPAQUE,
        }
    }

    fn expand565(px: u16) -> [u8; 3] {
        let r = ((px >> 11) & 0x1F) as u8;
        let g = ((px >> 5) & 0x3F) as u8;
        let b = (px & 0x1F) as u8;
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
    }

    fn quant565(r: u8, g: u8, b: u8) -> u16 {
        (((r as u16) & 0xF8) << 8) | (((g as u16) & 0xFC) << 3) | (((b as u16) & 0xF8) >> 3)
    }

    /// RGBA export: opaque -> expanded 565 + alpha 255; every other class ->
    /// (0,0,0,0). Keys ship in the lossless class map instead.
    fn sprite_rgba(s: &Sel) -> Vec<u8> {
        let mut rgba = vec![0u8; s.pixels.len() * 4];
        for (i, &px) in s.pixels.iter().enumerate() {
            if s.classes[i] == CL_OPAQUE {
                let [r, g, b] = expand565(px);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        rgba
    }

    /// 2-bit class map, packed 4 px/byte row-major.
    fn class_map_bits(s: &Sel) -> Vec<u8> {
        let mut out = vec![0u8; s.pixels.len().div_ceil(4)];
        for (i, &c) in s.classes.iter().enumerate() {
            out[i / 4] |= c << ((i % 4) * 2);
        }
        out
    }

    fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        let file =
            std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().context("png header")?;
        writer.write_image_data(rgba).context("png data")?;
        Ok(())
    }

    fn run_cjxl(
        cjxl: &str,
        input: &Path,
        output: &Path,
        quality: &[&str],
        effort: u8,
    ) -> Result<u64> {
        let out = std::process::Command::new(cjxl)
            .arg(input)
            .arg(output)
            .args(quality)
            .args(["-e", &effort.to_string(), "--quiet"])
            .output()
            .with_context(|| format!("spawn {cjxl}"))?;
        if !out.status.success() {
            bail!(
                "cjxl failed on {} ({}): {}",
                input.display(),
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(std::fs::metadata(output)?.len())
    }

    /// Decode a JXL blob to (width, height, RGBA8) via jxl-rs — the same
    /// decoder the runtime uses for terrain maps.
    fn decode_jxl_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
        use jxl::api::{
            JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
            JxlPixelFormat, ProcessingResult, states,
        };
        let mut input: &[u8] = bytes;
        let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());
        let mut dec_with_image = match dec.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated header"),
            Err(e) => bail!("jxl: header error: {e:?}"),
        };
        let (w, h) = dec_with_image.basic_info().size;
        // Fully-opaque sources (pak pictures) may come back without an alpha
        // extra channel — cjxl drops a constant-255 alpha.
        let has_alpha = !dec_with_image.basic_info().extra_channels.is_empty();
        dec_with_image.set_pixel_format(JxlPixelFormat {
            color_type: if has_alpha {
                JxlColorType::Rgba
            } else {
                JxlColorType::Rgb
            },
            color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
            extra_channel_format: if has_alpha { vec![None] } else { vec![] },
        });
        let dec_with_frame = match dec_with_image.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame header"),
            Err(e) => bail!("jxl: frame header error: {e:?}"),
        };
        let ch = if has_alpha { 4 } else { 3 };
        let stride = w * ch;
        let mut pixels = vec![0u8; stride * h];
        let mut bufs = vec![JxlOutputBuffer::new(&mut pixels, h, stride)];
        match dec_with_frame.process(&mut input, &mut bufs, None) {
            Ok(ProcessingResult::Complete { .. }) => {}
            Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame"),
            Err(e) => bail!("jxl: frame error: {e:?}"),
        }
        drop(bufs);
        let rgba = if has_alpha {
            pixels
        } else {
            let mut rgba = vec![255u8; w * h * 4];
            for i in 0..w * h {
                rgba[i * 4..i * 4 + 3].copy_from_slice(&pixels[i * 3..i * 3 + 3]);
            }
            rgba
        };
        Ok((w, h, rgba))
    }

    /// Quality of one decoded lossy sprite vs the original 565 pixels,
    /// opaque pixels only (keys come from the lossless class map).
    #[derive(Default, Clone, Copy)]
    struct QualityStats {
        opaque_px: u64,
        /// Sum of squared 8-bit channel errors over opaque pixels.
        sse: f64,
        /// Opaque pixels whose requantised RGB565 equals the original.
        exact565: u64,
        /// Opaque pixels whose requantised RGB565 lands exactly on a key.
        key_collisions: u64,
    }

    impl QualityStats {
        fn add(&mut self, o: &QualityStats) {
            self.opaque_px += o.opaque_px;
            self.sse += o.sse;
            self.exact565 += o.exact565;
            self.key_collisions += o.key_collisions;
        }
        fn psnr(&self) -> f64 {
            if self.opaque_px == 0 || self.sse == 0.0 {
                return f64::INFINITY;
            }
            let mse = self.sse / (self.opaque_px as f64 * 3.0);
            10.0 * (255.0f64 * 255.0 / mse).log10()
        }
    }

    fn score_decoded(s: &Sel, rgba: &[u8]) -> QualityStats {
        let mut q = QualityStats::default();
        for (i, &px) in s.pixels.iter().enumerate() {
            if s.classes[i] != CL_OPAQUE {
                continue;
            }
            q.opaque_px += 1;
            let [orig_r, orig_g, orig_b] = expand565(px);
            let (r, g, b) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
            for (a, o) in [(r, orig_r), (g, orig_g), (b, orig_b)] {
                let d = a as f64 - o as f64;
                q.sse += d * d;
            }
            let q565 = quant565(r, g, b);
            if q565 == px {
                q.exact565 += 1;
            }
            if q565 == TRANSPARENT_COLOR_16 || q565 == SHADOW_KEY {
                q.key_collisions += 1;
            }
        }
        q
    }

    /// Side-by-side dump (original | decoded) for visual inspection.
    /// Transparent renders mid-gray, shadow renders pure blue in BOTH
    /// halves (keys are exact by construction); opaque shows real error.
    fn dump_side_by_side(path: &Path, s: &Sel, decoded_rgba: &[u8]) -> Result<()> {
        let (w, h) = (s.width as usize, s.height as usize);
        let gap = 4usize;
        let out_w = w * 2 + gap;
        let mut rgba = vec![0u8; out_w * h * 4];
        let mut put = |x: usize, y: usize, p: [u8; 4]| {
            let off = (y * out_w + x) * 4;
            rgba[off..off + 4].copy_from_slice(&p);
        };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let px = s.pixels[i];
                let (orig, dec) = match s.classes[i] {
                    CL_TRANS => ([96, 96, 96, 255], [96, 96, 96, 255]),
                    CL_SHADOW => ([0, 0, 255, 255], [0, 0, 255, 255]),
                    CL_KEYLIT => ([255, 0, 255, 255], [255, 0, 255, 255]),
                    _ => {
                        let [r, g, b] = expand565(px);
                        (
                            [r, g, b, 255],
                            [
                                decoded_rgba[i * 4],
                                decoded_rgba[i * 4 + 1],
                                decoded_rgba[i * 4 + 2],
                                255,
                            ],
                        )
                    }
                };
                put(x, y, orig);
                put(w + gap + x, y, dec);
            }
        }
        write_png(path, out_w as u32, h as u32, &rgba)
    }

    fn fmt_bytes(b: u64) -> String {
        if b >= 10 * 1024 * 1024 {
            format!("{:.2} MiB", b as f64 / (1024.0 * 1024.0))
        } else if b >= 10 * 1024 {
            format!("{:.1} KiB", b as f64 / 1024.0)
        } else {
            format!("{b} B")
        }
    }

    /// Per-character result row for the final summary.
    struct Row {
        name: String,
        n_sel: usize,
        tiles_sel: u64,
        codec_share: u64,
        codec_sel: u64,
        jxl_q90: u64,
        jxl_q80: u64,
        jxl_d0: u64,
        mask_z: u64,
    }

    pub fn run() -> Result<()> {
        let args = parse_args()?;
        std::fs::create_dir_all(&args.out)?;

        // cjxl sanity probe before spending minutes on the bank load.
        {
            let probe_png = args.out.join("_probe.png");
            let probe_jxl = args.out.join("_probe.jxl");
            write_png(&probe_png, 4, 4, &[128u8; 64])?;
            run_cjxl(&args.cjxl, &probe_png, &probe_jxl, &["-q", "90"], 7)
                .context("cjxl sanity check failed — pass --cjxl <path>")?;
            let _ = std::fs::remove_file(probe_png);
            let _ = std::fs::remove_file(probe_jxl);
        }

        if args.pak || args.rle {
            let mut report = String::new();
            if args.pak {
                run_paks(&args, &mut report)?;
            }
            if args.rle {
                let t = Instant::now();
                let holder = FrameHolder::from_data_dir(&args.data_dir)?;
                println!(
                    "bank loaded: {} sprites, {} dictionaries ({:.1} s)",
                    holder.num_sprites(),
                    holder.dictionaries().len(),
                    t.elapsed().as_secs_f64()
                );
                run_rle(&args, &holder, &mut report)?;
            }
            let name = if args.rle {
                "report_rle.txt"
            } else {
                "report_pak.txt"
            };
            std::fs::write(args.out.join(name), &report)?;
            println!("\nfull report: {}", args.out.join(name).display());
            return Ok(());
        }

        let t = Instant::now();
        let holder = FrameHolder::from_data_dir(&args.data_dir)?;
        println!(
            "bank loaded: {} sprites, {} dictionaries ({:.1} s)",
            holder.num_sprites(),
            holder.dictionaries().len(),
            t.elapsed().as_secs_f64()
        );

        let mut rows: Vec<Row> = Vec::new();
        let mut report = String::new();
        for rel in &args.rhs {
            let row = probe_character(&args, &holder, rel, &mut report)?;
            rows.push(row);
        }

        // Cross-character summary.
        let mut s = String::new();
        writeln!(
            s,
            "\n== summary (selected sprites only; mask = zstd'd 2-bit class maps, \
             required for exact keys on the JXL side) =="
        )?;
        writeln!(
            s,
            "{:<28} {:>6} {:>10} | {:>10} {:>10} | {:>10} {:>10} {:>10} {:>10}",
            "character",
            "n_sel",
            "tiles",
            "codec-shr",
            "codec-sel",
            "jxl-q90",
            "jxl-q80",
            "jxl-d0",
            "mask+z",
        )?;
        for r in &rows {
            writeln!(
                s,
                "{:<28} {:>6} {:>10} | {:>10} {:>10} | {:>10} {:>10} {:>10} {:>10}",
                r.name,
                r.n_sel,
                r.tiles_sel,
                fmt_bytes(r.codec_share),
                fmt_bytes(r.codec_sel),
                fmt_bytes(r.jxl_q90),
                fmt_bytes(r.jxl_q80),
                fmt_bytes(r.jxl_d0),
                fmt_bytes(r.mask_z),
            )?;
        }
        writeln!(
            s,
            "{:<28} {:>6} {:>10} | {:>10} {:>10} | {:>10} {:>10} {:>10} {:>10}",
            "TOTAL",
            rows.iter().map(|r| r.n_sel).sum::<usize>(),
            rows.iter().map(|r| r.tiles_sel).sum::<u64>(),
            fmt_bytes(rows.iter().map(|r| r.codec_share).sum()),
            fmt_bytes(rows.iter().map(|r| r.codec_sel).sum()),
            fmt_bytes(rows.iter().map(|r| r.jxl_q90).sum()),
            fmt_bytes(rows.iter().map(|r| r.jxl_q80).sum()),
            fmt_bytes(rows.iter().map(|r| r.jxl_d0).sum()),
            fmt_bytes(rows.iter().map(|r| r.mask_z).sum()),
        )?;
        let codec: u64 = rows.iter().map(|r| r.codec_sel).sum();
        let q90: u64 = rows.iter().map(|r| r.jxl_q90 + r.mask_z).sum();
        let q80: u64 = rows.iter().map(|r| r.jxl_q80 + r.mask_z).sum();
        writeln!(
            s,
            "ratio vs codec-sel: jxl-q90+mask {:.2}x, jxl-q80+mask {:.2}x",
            q90 as f64 / codec as f64,
            q80 as f64 / codec as f64
        )?;
        print!("{s}");
        report.push_str(&s);
        std::fs::write(args.out.join("report.txt"), &report)?;
        println!("\nfull report: {}", args.out.join("report.txt").display());
        Ok(())
    }

    fn probe_character(
        args: &Args,
        holder: &FrameHolder,
        rel: &str,
        report: &mut String,
    ) -> Result<Row> {
        let mut out = String::new();
        writeln!(out, "\n== {rel} ==")?;
        let path = format!("{}/Data/{}", args.data_dir, rel);
        let (_sig, profiles) =
            SpriteScriptor::load_all_profiles(&path).map_err(|e| anyhow!("load {path}: {e}"))?;

        let mut used: BTreeSet<u32> = BTreeSet::new();
        for (_name, info) in &profiles {
            for s in info.scripts.iter() {
                used.extend(s.frame_ids.iter().copied());
            }
        }

        // Partition the used ids: well-formed VQ vs everything else.
        let mut all_vq: Vec<(u32, Vec<u16>)> = Vec::new(); // (id, grid)
        let mut alphabet: u16 = 0;
        let (mut n_rle, mut n_ragged, mut n_empty) = (0usize, 0usize, 0usize);
        for &id in &used {
            let sprite = holder
                .sprites()
                .get(id as usize)
                .ok_or_else(|| anyhow!("{rel}: sprite {id} beyond bank"))?;
            let (w, h) = (sprite.width, sprite.height);
            if w == 0 || h == 0 {
                n_empty += 1;
                continue;
            }
            if sprite.dictionary_index == UNMAPPED_DICT {
                n_rle += 1;
                continue;
            }
            let packed = holder
                .packed_data(id)
                .ok_or_else(|| anyhow!("{rel}: sprite {id} has no packed data"))?;
            if w % 4 != 0 || packed.len() != (w as usize / 4) * h as usize {
                n_ragged += 1;
                continue;
            }
            let dict = holder
                .dictionary(sprite.dictionary_index)
                .ok_or_else(|| anyhow!("{rel}: sprite {id} missing dictionary"))?;
            alphabet = alphabet.max(dict.num_entries());
            all_vq.push((id, packed.to_vec()));
        }

        let mut selected: Vec<Sel> = Vec::new();
        for (id, grid) in &all_vq {
            let sprite = &holder.sprites()[*id as usize];
            let (w, h) = (sprite.width, sprite.height);
            if w < args.min_dim || h < args.min_dim {
                continue;
            }
            if args.limit > 0 && selected.len() >= args.limit {
                break;
            }
            let mut pixels = vec![0u16; w as usize * h as usize];
            holder.uncompress_frame(
                &mut pixels,
                w as usize,
                *id,
                SpriteVariant::Day,
                SHADOW_KEY,
                16,
            );
            let classes = pixels.iter().map(|&p| classify(p)).collect();
            selected.push(Sel {
                id: *id,
                width: w,
                height: h,
                grid: grid.clone(),
                pixels,
                classes,
            });
        }

        let tiles_all: u64 = all_vq.iter().map(|(_, g)| g.len() as u64).sum();
        let tiles_sel: u64 = selected.iter().map(|s| s.grid.len() as u64).sum();
        writeln!(
            out,
            "profiles {}, used sprites {}, VQ {} (skipped: {n_rle} rle, {n_ragged} ragged, \
             {n_empty} empty)",
            profiles.len(),
            used.len(),
            all_vq.len(),
        )?;
        writeln!(
            out,
            "selected (>= {0}x{0} px): {1} sprites, {2} tiles of {3} total VQ tiles \
             ({4:.1}% of tiles)",
            args.min_dim,
            selected.len(),
            tiles_sel,
            tiles_all,
            100.0 * tiles_sel as f64 / tiles_all.max(1) as f64,
        )?;
        if selected.is_empty() {
            bail!("{rel}: no sprites selected — wrong RHS for this probe");
        }

        // -- Current codec comparators ---------------------------------
        let encode_set = |set: &[(u32, &[u16])]| -> Result<(Vec<u8>, f64, f64)> {
            let ids: Vec<u32> = set.iter().map(|(id, _)| *id).collect();
            let selfrefs = derive_chunk_self_refs(&profiles, &ids);
            let grids: Vec<SpriteGrid> = set
                .iter()
                .map(|(id, grid)| {
                    let s = &holder.sprites()[*id as usize];
                    SpriteGrid {
                        cols: s.width / 4,
                        rows: s.height,
                        indices: grid,
                    }
                })
                .collect();
            let t = Instant::now();
            let blob =
                sprite_codec::encode_grids_shipping(alphabet, &grids, None, None, &selfrefs)?;
            let enc_s = t.elapsed().as_secs_f64();
            let dims: Vec<(u16, u16)> = grids.iter().map(|g| (g.cols, g.rows)).collect();
            let t = Instant::now();
            let decoded =
                sprite_codec::decode_grids_shipping(alphabet, &dims, None, None, &selfrefs, &blob)?;
            let dec_s = t.elapsed().as_secs_f64();
            for (g, d) in grids.iter().zip(&decoded) {
                if g.indices != d.as_slice() {
                    bail!("codec roundtrip mismatch");
                }
            }
            Ok((blob, enc_s, dec_s))
        };

        let all_refs: Vec<(u32, &[u16])> =
            all_vq.iter().map(|(id, g)| (*id, g.as_slice())).collect();
        let (blob_all, enc_all_s, dec_all_s) = encode_set(&all_refs)?;
        let sel_refs: Vec<(u32, &[u16])> =
            selected.iter().map(|s| (s.id, s.grid.as_slice())).collect();
        let (blob_sel, _enc_sel_s, dec_sel_s) = encode_set(&sel_refs)?;
        let codec_share = (blob_all.len() as f64 * tiles_sel as f64 / tiles_all as f64) as u64;
        writeln!(
            out,
            "codec blob(all VQ)   = {} (alphabet {}, encode {:.1} s, decode {:.2} s, \
             roundtrip ok)",
            fmt_bytes(blob_all.len() as u64),
            alphabet,
            enc_all_s,
            dec_all_s,
        )?;
        writeln!(
            out,
            "codec share of sel   = {} (prorated by tiles)",
            fmt_bytes(codec_share)
        )?;
        writeln!(
            out,
            "codec blob(sel only) = {} (exact comparator, decode {:.2} s)",
            fmt_bytes(blob_sel.len() as u64),
            dec_sel_s,
        )?;

        // -- JXL per-sprite --------------------------------------------
        let stem = rel.trim_end_matches(".rhs").replace(['/', ' '], "_");
        let char_dir = args.out.join(&stem);
        let png_dir = char_dir.join("png");
        for q in ["q90", "q80", "d0", "worst_q90", "worst_q80"] {
            std::fs::create_dir_all(char_dir.join(q))?;
        }
        std::fs::create_dir_all(&png_dir)?;

        let t = Instant::now();
        // (id, q90, q80, d0) sizes.
        let per_sprite: Vec<(u32, u64, u64, u64)> = selected
            .par_iter()
            .map(|s| -> Result<(u32, u64, u64, u64)> {
                let png_path = png_dir.join(format!("{}.png", s.id));
                write_png(&png_path, s.width as u32, s.height as u32, &sprite_rgba(s))?;
                let q90 = run_cjxl(
                    &args.cjxl,
                    &png_path,
                    &char_dir.join("q90").join(format!("{}.jxl", s.id)),
                    &["-q", "90"],
                    7,
                )?;
                let q80 = run_cjxl(
                    &args.cjxl,
                    &png_path,
                    &char_dir.join("q80").join(format!("{}.jxl", s.id)),
                    &["-q", "80"],
                    7,
                )?;
                let d0 = run_cjxl(
                    &args.cjxl,
                    &png_path,
                    &char_dir.join("d0").join(format!("{}.jxl", s.id)),
                    &["-d", "0"],
                    7,
                )?;
                if !args.keep_pngs {
                    let _ = std::fs::remove_file(&png_path);
                }
                Ok((s.id, q90, q80, d0))
            })
            .collect::<Result<Vec<_>>>()?;
        let encode_s = t.elapsed().as_secs_f64();
        let jxl_q90: u64 = per_sprite.iter().map(|r| r.1).sum();
        let jxl_q80: u64 = per_sprite.iter().map(|r| r.2).sum();
        let jxl_d0: u64 = per_sprite.iter().map(|r| r.3).sum();

        // Class-map sidecar: concatenated 2-bit maps, shipping-grade zstd.
        let mut masks = Vec::new();
        for s in &selected {
            masks.extend_from_slice(&class_map_bits(s));
        }
        let mask_z = robin_assets::shipping_datadir::zstd_max_compress(&masks)?.len() as u64;
        writeln!(
            out,
            "jxl per-sprite sums  = q90 {} | q80 {} | d0(lossless) {} | class-mask+zstd {} \
             (cjxl e7, {:.0} s wall on {} threads)",
            fmt_bytes(jxl_q90),
            fmt_bytes(jxl_q80),
            fmt_bytes(jxl_d0),
            fmt_bytes(mask_z),
            encode_s,
            rayon::current_num_threads(),
        )?;
        writeln!(
            out,
            "ratio vs codec-sel   = q90+mask {:.2}x | q80+mask {:.2}x | d0+mask {:.2}x",
            (jxl_q90 + mask_z) as f64 / blob_sel.len() as f64,
            (jxl_q80 + mask_z) as f64 / blob_sel.len() as f64,
            (jxl_d0 + mask_z) as f64 / blob_sel.len() as f64,
        )?;

        // -- Quality + decode timing (jxl-rs, single thread) -----------
        for quality in ["q90", "q80"] {
            let dir = char_dir.join(quality);
            let mut agg = QualityStats::default();
            let mut per: Vec<(f64, u32)> = Vec::with_capacity(selected.len());
            let mut decode_s = 0.0f64;
            for s in &selected {
                let bytes = std::fs::read(dir.join(format!("{}.jxl", s.id)))?;
                let t = Instant::now();
                let (w, h, rgba) = decode_jxl_rgba(&bytes)?;
                decode_s += t.elapsed().as_secs_f64();
                if (w, h) != (s.width as usize, s.height as usize) {
                    bail!("jxl decode size mismatch for sprite {}", s.id);
                }
                let q = score_decoded(s, &rgba);
                per.push((q.psnr(), s.id));
                agg.add(&q);
            }
            per.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut worst_note = String::new();
            for &(psnr, id) in per.iter().take(5) {
                let s = selected.iter().find(|s| s.id == id).unwrap();
                let bytes = std::fs::read(dir.join(format!("{id}.jxl")))?;
                let (_, _, rgba) = decode_jxl_rgba(&bytes)?;
                let dump = char_dir
                    .join(format!("worst_{quality}"))
                    .join(format!("{id}_psnr{psnr:.1}.png"));
                dump_side_by_side(&dump, s, &rgba)?;
                write!(worst_note, " {id}:{psnr:.1}dB")?;
            }
            writeln!(
                out,
                "{quality}: PSNR {:.1} dB opaque-px (worst{worst_note}), 565-exact {:.1}%, \
                 key-collisions {} / {} px, jxl-rs decode {:.2} s for {} images \
                 ({:.2} ms/img, 1 thread)",
                agg.psnr(),
                100.0 * agg.exact565 as f64 / agg.opaque_px.max(1) as f64,
                agg.key_collisions,
                agg.opaque_px,
                decode_s,
                selected.len(),
                1000.0 * decode_s / selected.len() as f64,
            )?;
        }

        // -- Atlas variant ---------------------------------------------
        // The (profile, action) whose rows reference the most DISTINCT
        // selected sprites; frames packed once into a grid atlas.
        let sel_ids: BTreeSet<u32> = selected.iter().map(|s| s.id).collect();
        let mut best: Option<(String, u16, Vec<u32>)> = None;
        for (name, info) in &profiles {
            let mut by_action: std::collections::BTreeMap<u16, Vec<u32>> = Default::default();
            for s in info.scripts.iter() {
                let e = by_action.entry(s.action_id).or_default();
                for &id in &s.frame_ids {
                    if sel_ids.contains(&id) && !e.contains(&id) {
                        e.push(id);
                    }
                }
            }
            for (action, ids) in by_action {
                if best.as_ref().is_none_or(|(_, _, b)| ids.len() > b.len()) && !ids.is_empty() {
                    best = Some((name.clone(), action, ids));
                }
            }
        }
        if let Some((profile, action, ids)) = best {
            atlas_variant(
                args,
                holder,
                &profiles,
                alphabet,
                &selected,
                &per_sprite,
                &char_dir,
                &profile,
                action,
                &ids,
                &mut out,
            )?;
        }

        println!("{out}");
        report.push_str(&out);
        Ok(Row {
            name: rel.to_owned(),
            n_sel: selected.len(),
            tiles_sel,
            codec_share,
            codec_sel: blob_sel.len() as u64,
            jxl_q90,
            jxl_q80,
            jxl_d0,
            mask_z,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn atlas_variant(
        args: &Args,
        holder: &FrameHolder,
        profiles: &[(String, SpriteInfo)],
        alphabet: u16,
        selected: &[Sel],
        per_sprite: &[(u32, u64, u64, u64)],
        char_dir: &Path,
        profile: &str,
        action: u16,
        ids: &[u32],
        out: &mut String,
    ) -> Result<()> {
        let frames: Vec<&Sel> = ids
            .iter()
            .map(|id| selected.iter().find(|s| s.id == *id).unwrap())
            .collect();
        let (cols, rows, aw, ah, rgba) = pack_atlas_rgba(&frames);
        let atlas_png = char_dir.join("atlas.png");
        write_png(&atlas_png, aw as u32, ah as u32, &rgba)?;
        let a90 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_q90.jxl"),
            &["-q", "90"],
            7,
        )?;
        let a80 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_q80.jxl"),
            &["-q", "80"],
            7,
        )?;
        let a0 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_d0.jxl"),
            &["-d", "0"],
            7,
        )?;

        let in_set: BTreeSet<u32> = ids.iter().copied().collect();
        let sum = |f: fn(&(u32, u64, u64, u64)) -> u64| -> u64 {
            per_sprite
                .iter()
                .filter(|r| in_set.contains(&r.0))
                .map(f)
                .sum()
        };
        let (p90, p80, p0) = (sum(|r| r.1), sum(|r| r.2), sum(|r| r.3));

        // Codec exact comparator over the same frames (ascending id order,
        // like the shipping chunk).
        let mut sorted: Vec<&Sel> = frames.clone();
        sorted.sort_by_key(|s| s.id);
        let set: Vec<(u32, &[u16])> = sorted.iter().map(|s| (s.id, s.grid.as_slice())).collect();
        let sel_ids_sorted: Vec<u32> = set.iter().map(|(id, _)| *id).collect();
        let selfrefs = derive_chunk_self_refs(profiles, &sel_ids_sorted);
        let grids: Vec<SpriteGrid> = set
            .iter()
            .map(|(id, grid)| {
                let s = &holder.sprites()[*id as usize];
                SpriteGrid {
                    cols: s.width / 4,
                    rows: s.height,
                    indices: grid,
                }
            })
            .collect();
        let blob = sprite_codec::encode_grids_shipping(alphabet, &grids, None, None, &selfrefs)?;

        writeln!(
            out,
            "atlas [{profile}:act{action}, {} frames, {}x{} cells {}x{} px]: \
             q90 {} | q80 {} | d0 {}   (same frames per-sprite: q90 {} | q80 {} | d0 {}; \
             codec {})",
            frames.len(),
            cols,
            rows,
            aw,
            ah,
            fmt_bytes(a90),
            fmt_bytes(a80),
            fmt_bytes(a0),
            fmt_bytes(p90),
            fmt_bytes(p80),
            fmt_bytes(p0),
            fmt_bytes(blob.len() as u64),
        )?;
        Ok(())
    }

    /// Grid-atlas packing shared by the VQ atlas variant and `--rle`:
    /// uniform cells of the max frame dims, sqrt-ish layout, frames blitted
    /// top-left. Returns `(cols, rows, atlas_w, atlas_h, rgba)`.
    fn pack_atlas_rgba(frames: &[&Sel]) -> (usize, usize, usize, usize, Vec<u8>) {
        let cell_w = frames.iter().map(|s| s.width as usize).max().unwrap();
        let cell_h = frames.iter().map(|s| s.height as usize).max().unwrap();
        let cols = (frames.len() as f64).sqrt().ceil() as usize;
        let rows = frames.len().div_ceil(cols);
        let (aw, ah) = (cols * cell_w, rows * cell_h);
        let mut rgba = vec![0u8; aw * ah * 4];
        for (k, s) in frames.iter().enumerate() {
            let (ox, oy) = ((k % cols) * cell_w, (k / cols) * cell_h);
            let src = sprite_rgba(s);
            for y in 0..s.height as usize {
                let dst_off = ((oy + y) * aw + ox) * 4;
                let src_off = y * s.width as usize * 4;
                let n = s.width as usize * 4;
                rgba[dst_off..dst_off + n].copy_from_slice(&src[src_off..src_off + n]);
            }
        }
        (cols, rows, aw, ah, rgba)
    }

    // ==================================================================
    // --rle mode: the RLE/patch bucket (ambient animations, map patches)
    // ==================================================================

    fn data_subdir(data_dir: &str, sub: &str) -> Result<PathBuf> {
        for case in ["Data", "DATA"] {
            let p = Path::new(data_dir).join(case).join(sub);
            if p.is_dir() {
                return Ok(p);
            }
        }
        bail!("no {sub} under {data_dir}/(Data|DATA)")
    }

    fn rhs_files_with_prefixes(dir: &Path, prefixes: &[&str]) -> Result<Vec<PathBuf>> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| {
                p.extension().is_some_and(|e| e == "rhs")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| prefixes.iter().any(|pre| n.starts_with(pre)))
            })
            .collect();
        files.sort();
        Ok(files)
    }

    fn rhs_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| Some(e.ok()?.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                rhs_files_recursive(&p, out)?;
            } else if p.extension().is_some_and(|e| e == "rhs") {
                out.push(p);
            }
        }
        Ok(())
    }

    /// Decode one RLE sprite to canvas pixels + classes. Returns the words
    /// consumed — callers count sprites whose packed stream carries trailing
    /// words beyond the walk (they exist in the bank; a canvas
    /// representation drops them).
    fn decode_rle_canvas(
        width: usize,
        height: usize,
        packed: &[u16],
    ) -> Result<(Vec<u16>, Vec<u8>, usize)> {
        let mut pixels = vec![TRANSPARENT_COLOR_16; width * height];
        let mut classes = vec![CL_TRANS; width * height];
        let mut p = 0usize;
        for y in 0..height {
            let first = *packed.get(p).ok_or_else(|| anyhow!("rle: truncated ctl"))?;
            let last = *packed
                .get(p + 1)
                .ok_or_else(|| anyhow!("rle: truncated ctl"))?;
            p += 2;
            if last == 0xFFFF {
                continue;
            }
            let (first, last) = (first as usize, last as usize);
            if first > last || last >= width {
                bail!("rle: bad run {first}..={last} in width {width}");
            }
            let run = last + 1 - first;
            let lits = packed
                .get(p..p + run)
                .ok_or_else(|| anyhow!("rle: truncated literals"))?;
            for (k, &c) in lits.iter().enumerate() {
                let i = y * width + first + k;
                pixels[i] = c;
                classes[i] = match c {
                    SHADOW_KEY => CL_SHADOW,
                    TRANSPARENT_COLOR_16 => CL_KEYLIT,
                    _ => CL_OPAQUE,
                };
            }
            p += run;
        }
        Ok((pixels, classes, p))
    }

    /// One (rhs, profile, action) animation group of selected sprite ids.
    struct RleGroup {
        label: String,
        ids: Vec<u32>,
    }

    #[derive(Default)]
    struct RleSet {
        selected: Vec<Sel>,
        /// Ledger corpus blob (w,h,dict,len,packed) of ALL claimed
        /// well-formed RLE sprites of this set, including sub-min-dim ones.
        blob_all: Vec<u8>,
        n_all: usize,
        n_small: usize,
        n_vq: usize,
        n_files: usize,
        n_failed: usize,
        n_trailing: usize,
    }

    /// Load every file's scripts; each bank id is claimed by the first
    /// (file, profile, action) group that references it — the ledger's
    /// first-claim dedup, extended with group membership for atlases.
    fn gather_rle_set(
        args: &Args,
        holder: &FrameHolder,
        files: &[PathBuf],
        claimed: &mut HashSet<u32>,
        groups: &mut Vec<RleGroup>,
    ) -> Result<RleSet> {
        let mut set = RleSet {
            n_files: files.len(),
            ..Default::default()
        };
        for f in files {
            let Ok((_sig, profiles)) = SpriteScriptor::load_all_profiles(
                f.to_str().ok_or_else(|| anyhow!("non-UTF8 path"))?,
            ) else {
                eprintln!("# failed to load {}", f.display());
                set.n_failed += 1;
                continue;
            };
            let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
            let mut by_group: BTreeMap<(String, u16), Vec<u32>> = BTreeMap::new();
            for (pname, info) in &profiles {
                for s in info.scripts.iter() {
                    let e = by_group.entry((pname.clone(), s.action_id)).or_default();
                    for &id in &s.frame_ids {
                        if id as usize >= holder.num_sprites() {
                            continue;
                        }
                        if claimed.insert(id) {
                            e.push(id);
                        }
                    }
                }
            }
            for ((pname, action), ids) in by_group {
                let mut sel_ids = Vec::new();
                for id in ids {
                    let sprite = &holder.sprites()[id as usize];
                    let (w, h) = (sprite.width, sprite.height);
                    if w == 0 || h == 0 {
                        continue;
                    }
                    if sprite.dictionary_index != UNMAPPED_DICT {
                        set.n_vq += 1;
                        continue;
                    }
                    let Some(packed) = holder.packed_data(id) else {
                        continue;
                    };
                    set.n_all += 1;
                    set.blob_all.extend_from_slice(&w.to_le_bytes());
                    set.blob_all.extend_from_slice(&h.to_le_bytes());
                    set.blob_all
                        .extend_from_slice(&sprite.dictionary_index.to_le_bytes());
                    set.blob_all
                        .extend_from_slice(&(packed.len() as u32).to_le_bytes());
                    for wd in packed {
                        set.blob_all.extend_from_slice(&wd.to_le_bytes());
                    }
                    if w < args.min_dim || h < args.min_dim {
                        set.n_small += 1;
                        continue;
                    }
                    if args.limit > 0 && set.selected.len() >= args.limit {
                        continue;
                    }
                    let (pixels, classes, used) = decode_rle_canvas(w as usize, h as usize, packed)
                        .with_context(|| format!("sprite {id}"))?;
                    if used != packed.len() {
                        set.n_trailing += 1;
                    }
                    sel_ids.push(id);
                    set.selected.push(Sel {
                        id,
                        width: w,
                        height: h,
                        grid: packed.to_vec(),
                        pixels,
                        classes,
                    });
                }
                if !sel_ids.is_empty() {
                    groups.push(RleGroup {
                        label: format!("{stem}:{pname}:act{action}"),
                        ids: sel_ids,
                    });
                }
            }
        }
        Ok(set)
    }

    /// zstd level 19 (the ledger baseline): (compressed len, enc s, dec s).
    fn zstd19_stats(data: &[u8]) -> Result<(u64, f64, f64)> {
        let t = Instant::now();
        let z = zstd::stream::encode_all(data, 19)?;
        let enc_s = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let d = zstd::stream::decode_all(&z[..])?;
        let dec_s = t.elapsed().as_secs_f64();
        if d.len() != data.len() {
            bail!("zstd roundtrip length mismatch");
        }
        Ok((z.len() as u64, enc_s, dec_s))
    }

    /// xz -9e via the CLI (the ledger tool): (compressed len, enc s, dec s).
    /// Temp files live under the probe's own out dir (worktree-safe).
    fn xz9e_stats(out_dir: &Path, label: &str, data: &[u8]) -> Result<(u64, f64, f64)> {
        let raw = out_dir.join(format!("xz_{label}.bin"));
        let packed = out_dir.join(format!("xz_{label}.xz"));
        std::fs::write(&raw, data)?;
        let t = Instant::now();
        let out = std::process::Command::new("xz")
            .args(["-9e", "-T1", "-c"])
            .arg(&raw)
            .output()
            .context("run xz -9e (is xz installed?)")?;
        let enc_s = t.elapsed().as_secs_f64();
        if !out.status.success() {
            bail!("xz -9e failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        std::fs::write(&packed, &out.stdout)?;
        let t = Instant::now();
        let dec = std::process::Command::new("xz")
            .args(["-d", "-T1", "-c"])
            .arg(&packed)
            .output()
            .context("run xz -d")?;
        let dec_s = t.elapsed().as_secs_f64();
        if !dec.status.success() || dec.stdout.len() != data.len() {
            bail!("xz roundtrip failed");
        }
        let n = out.stdout.len() as u64;
        let _ = std::fs::remove_file(raw);
        let _ = std::fs::remove_file(packed);
        Ok((n, enc_s, dec_s))
    }

    /// Animated PNG of a group: every frame blitted top-left onto a
    /// max-dims canvas. cjxl accepts APNG input and carries the frame
    /// sequence into an animated JXL.
    fn write_apng(path: &Path, frames: &[&Sel]) -> Result<()> {
        let cell_w = frames.iter().map(|s| s.width as usize).max().unwrap();
        let cell_h = frames.iter().map(|s| s.height as usize).max().unwrap();
        let file = std::fs::File::create(path)?;
        let mut enc =
            png::Encoder::new(std::io::BufWriter::new(file), cell_w as u32, cell_h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, 0).context("apng")?;
        enc.set_frame_delay(1, 10).context("apng delay")?;
        let mut w = enc.write_header().context("png header")?;
        for s in frames {
            let mut canvas = vec![0u8; cell_w * cell_h * 4];
            let src = sprite_rgba(s);
            for y in 0..s.height as usize {
                let dst_off = y * cell_w * 4;
                let src_off = y * s.width as usize * 4;
                let n = s.width as usize * 4;
                canvas[dst_off..dst_off + n].copy_from_slice(&src[src_off..src_off + n]);
            }
            w.write_image_data(&canvas).context("apng frame")?;
        }
        w.finish().context("apng finish")?;
        Ok(())
    }

    const QUALITY_SET: [(&str, [&str; 2]); 4] = [
        ("q90", ["-q", "90"]),
        ("q80", ["-q", "80"]),
        ("q70", ["-q", "70"]),
        ("d0", ["-d", "0"]),
    ];

    fn run_rle(args: &Args, holder: &FrameHolder, report: &mut String) -> Result<()> {
        let mut out = String::new();
        writeln!(out, "\n== RLE/patch bucket ==")?;

        let chars_dir = data_subdir(&args.data_dir, "Characters")?;
        let acc_files =
            rhs_files_with_prefixes(&chars_dir, &["ACCESSORIES_", "BONUS_", "RELIC_", "TG_"])?;
        let mut anim_files = Vec::new();
        rhs_files_recursive(&data_subdir(&args.data_dir, "Animations")?, &mut anim_files)?;

        let mut claimed: HashSet<u32> = HashSet::new();
        let mut groups: Vec<RleGroup> = Vec::new();
        let acc = gather_rle_set(args, holder, &acc_files, &mut claimed, &mut groups)?;
        let anim = gather_rle_set(args, holder, &anim_files, &mut claimed, &mut groups)?;

        let raw_all = acc.blob_all.len() + anim.blob_all.len();
        writeln!(
            out,
            "gather: {} accessory rhs + {} animation rhs ({} failed); {} RLE sprites, \
             raw blob {} (ledger: 10,134 / 66.8 MB); {} VQ sprites skipped \
             (schema v9 blob-codes those)",
            acc.n_files,
            anim.n_files,
            acc.n_failed + anim.n_failed,
            acc.n_all + anim.n_all,
            fmt_bytes(raw_all as u64),
            acc.n_vq + anim.n_vq,
        )?;
        let selected: Vec<&Sel> = acc.selected.iter().chain(anim.selected.iter()).collect();
        let raw_sel: u64 = selected.iter().map(|s| 10 + 2 * s.grid.len() as u64).sum();
        let canvas_px: u64 = selected.iter().map(|s| s.pixels.len() as u64).sum();
        let opaque_px: u64 = selected
            .iter()
            .map(|s| s.classes.iter().filter(|&&c| c == CL_OPAQUE).count() as u64)
            .sum();
        writeln!(
            out,
            "selected (>= {0}x{0} px): {1} sprites ({2} accessory + {3} animation; {4} small \
             skipped), raw {5} = {6:.1}% of bucket, canvas {7} px ({8:.1}% opaque), \
             trailing-word sprites {9}",
            args.min_dim,
            selected.len(),
            acc.selected.len(),
            anim.selected.len(),
            acc.n_small + anim.n_small,
            fmt_bytes(raw_sel),
            100.0 * raw_sel as f64 / (raw_all as u64).max(1) as f64,
            canvas_px,
            100.0 * opaque_px as f64 / canvas_px.max(1) as f64,
            acc.n_trailing + anim.n_trailing,
        )?;
        if selected.is_empty() {
            bail!("no RLE sprites selected");
        }

        // -- entropy-coder baselines over the ledger blob layout ---------
        let mut blob_sel = Vec::with_capacity(raw_sel as usize);
        for s in &selected {
            blob_sel.extend_from_slice(&s.width.to_le_bytes());
            blob_sel.extend_from_slice(&s.height.to_le_bytes());
            blob_sel.extend_from_slice(&UNMAPPED_DICT.to_le_bytes());
            blob_sel.extend_from_slice(&(s.grid.len() as u32).to_le_bytes());
            for wd in &s.grid {
                blob_sel.extend_from_slice(&wd.to_le_bytes());
            }
        }
        let mut blob_all = acc.blob_all.clone();
        blob_all.extend_from_slice(&anim.blob_all);
        let (z_all, _, zd_all) = zstd19_stats(&blob_all)?;
        let (x_all, xe_all, xd_all) = xz9e_stats(&args.out, "rle_all", &blob_all)?;
        let (z_sel, _, zd_sel) = zstd19_stats(&blob_sel)?;
        let (x_sel, xe_sel, xd_sel) = xz9e_stats(&args.out, "rle_sel", &blob_sel)?;
        writeln!(
            out,
            "bucket baselines:   zstd-19 {} (inflate {:.2} s) | xz -9e {} (compress {:.0} s, \
             inflate {:.2} s)   [ledger full bucket: zstd-19 17.69 MB, xz 15.56 MB, \
             pixel-CM 16.65 MB]",
            fmt_bytes(z_all),
            zd_all,
            fmt_bytes(x_all),
            xe_all,
            xd_all,
        )?;
        writeln!(
            out,
            "selected baselines: zstd-19 {} (inflate {:.2} s) | xz -9e {} (compress {:.0} s, \
             inflate {:.2} s)  <- exact comparator for the JXL side",
            fmt_bytes(z_sel),
            zd_sel,
            fmt_bytes(x_sel),
            xe_sel,
            xd_sel,
        )?;

        // -- JXL per-sprite ---------------------------------------------
        let rle_dir = args.out.join("rle");
        let png_dir = rle_dir.join("png");
        for d in ["q90", "q80", "q70", "d0", "worst_q80", "worst_q70", "atlas"] {
            std::fs::create_dir_all(rle_dir.join(d))?;
        }
        std::fs::create_dir_all(&png_dir)?;

        let t = Instant::now();
        let per_sprite: HashMap<u32, [u64; 4]> = selected
            .par_iter()
            .map(|s| -> Result<(u32, [u64; 4])> {
                let png_path = png_dir.join(format!("{}.png", s.id));
                write_png(&png_path, s.width as u32, s.height as u32, &sprite_rgba(s))?;
                let mut sizes = [0u64; 4];
                for (k, (tag, qargs)) in QUALITY_SET.iter().enumerate() {
                    sizes[k] = run_cjxl(
                        &args.cjxl,
                        &png_path,
                        &rle_dir.join(tag).join(format!("{}.jxl", s.id)),
                        qargs,
                        7,
                    )?;
                }
                if !args.keep_pngs {
                    let _ = std::fs::remove_file(&png_path);
                }
                Ok((s.id, sizes))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let encode_s = t.elapsed().as_secs_f64();
        let sum_q = |k: usize| -> u64 { per_sprite.values().map(|v| v[k]).sum() };
        let (jq90, jq80, jq70, jd0) = (sum_q(0), sum_q(1), sum_q(2), sum_q(3));

        let mut masks = Vec::new();
        for s in &selected {
            masks.extend_from_slice(&class_map_bits(s));
        }
        let mask_z = robin_assets::shipping_datadir::zstd_max_compress(&masks)?.len() as u64;
        writeln!(
            out,
            "jxl per-sprite sums: q90 {} | q80 {} | q70 {} | d0(lossless) {} | \
             class-mask+zstd {} (cjxl e7, {:.0} s wall on {} threads)",
            fmt_bytes(jq90),
            fmt_bytes(jq80),
            fmt_bytes(jq70),
            fmt_bytes(jd0),
            fmt_bytes(mask_z),
            encode_s,
            rayon::current_num_threads(),
        )?;
        writeln!(
            out,
            "ratio vs xz-9e(sel): q90+mask {:.2}x | q80+mask {:.2}x | q70+mask {:.2}x | \
             d0+mask {:.2}x",
            (jq90 + mask_z) as f64 / x_sel as f64,
            (jq80 + mask_z) as f64 / x_sel as f64,
            (jq70 + mask_z) as f64 / x_sel as f64,
            (jd0 + mask_z) as f64 / x_sel as f64,
        )?;

        // -- quality + decode timing ------------------------------------
        for quality in ["q90", "q80", "q70"] {
            let dir = rle_dir.join(quality);
            let mut agg = QualityStats::default();
            let mut per: Vec<(f64, u32)> = Vec::with_capacity(selected.len());
            let mut decode_s = 0.0f64;
            for s in &selected {
                let bytes = std::fs::read(dir.join(format!("{}.jxl", s.id)))?;
                let t = Instant::now();
                let (w, h, rgba) = decode_jxl_rgba(&bytes)?;
                decode_s += t.elapsed().as_secs_f64();
                if (w, h) != (s.width as usize, s.height as usize) {
                    bail!("jxl decode size mismatch for sprite {}", s.id);
                }
                let q = score_decoded(s, &rgba);
                per.push((q.psnr(), s.id));
                agg.add(&q);
            }
            per.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut worst_note = String::new();
            for &(psnr, id) in per.iter().take(5) {
                write!(worst_note, " {id}:{psnr:.1}dB")?;
                if quality != "q90" {
                    let s = selected.iter().find(|s| s.id == id).unwrap();
                    let bytes = std::fs::read(dir.join(format!("{id}.jxl")))?;
                    let (_, _, rgba) = decode_jxl_rgba(&bytes)?;
                    let dump = rle_dir
                        .join(format!("worst_{quality}"))
                        .join(format!("{id}_psnr{psnr:.1}.png"));
                    dump_side_by_side(&dump, s, &rgba)?;
                }
            }
            writeln!(
                out,
                "{quality}: PSNR {:.1} dB opaque-px (worst{worst_note}), 565-exact {:.1}%, \
                 key-collisions {} / {} px, jxl-rs decode {:.2} s for {} images \
                 ({:.2} ms/img, 1 thread)",
                agg.psnr(),
                100.0 * agg.exact565 as f64 / agg.opaque_px.max(1) as f64,
                agg.key_collisions,
                agg.opaque_px,
                decode_s,
                selected.len(),
                1000.0 * decode_s / selected.len() as f64,
            )?;
        }

        // -- atlas + animated variants over animation groups -------------
        let by_id: HashMap<u32, &Sel> = selected.iter().map(|s| (s.id, *s)).collect();
        let atlas_groups: Vec<&RleGroup> = groups.iter().filter(|g| g.ids.len() >= 4).collect();
        let animated_ok = if let Some(g) = atlas_groups.first() {
            let frames: Vec<&Sel> = g.ids.iter().map(|id| by_id[id]).collect();
            let apng = rle_dir.join("atlas").join("_anim_probe.png");
            let ajxl = rle_dir.join("atlas").join("_anim_probe.jxl");
            write_apng(&apng, &frames)?;
            let ok = run_cjxl(&args.cjxl, &apng, &ajxl, &["-q", "80"], 7).is_ok();
            let _ = std::fs::remove_file(&apng);
            let _ = std::fs::remove_file(&ajxl);
            ok
        } else {
            false
        };

        let t = Instant::now();
        let group_res: Vec<([u64; 4], [u64; 2])> = atlas_groups
            .par_iter()
            .enumerate()
            .map(|(gi, g)| -> Result<([u64; 4], [u64; 2])> {
                let frames: Vec<&Sel> = g.ids.iter().map(|id| by_id[id]).collect();
                let (_c, _r, aw, ah, rgba) = pack_atlas_rgba(&frames);
                let png_path = rle_dir.join("atlas").join(format!("{gi}.png"));
                write_png(&png_path, aw as u32, ah as u32, &rgba)?;
                let mut sizes = [0u64; 4];
                for (k, (tag, qargs)) in QUALITY_SET.iter().enumerate() {
                    sizes[k] = run_cjxl(
                        &args.cjxl,
                        &png_path,
                        &rle_dir.join("atlas").join(format!("{gi}_{tag}.jxl")),
                        qargs,
                        7,
                    )?;
                }
                let _ = std::fs::remove_file(&png_path);
                let mut anim_sizes = [0u64; 2];
                if animated_ok {
                    let apng = rle_dir.join("atlas").join(format!("{gi}_anim.png"));
                    write_apng(&apng, &frames)?;
                    anim_sizes[0] = run_cjxl(
                        &args.cjxl,
                        &apng,
                        &rle_dir.join("atlas").join(format!("{gi}_anim_q80.jxl")),
                        &["-q", "80"],
                        7,
                    )?;
                    anim_sizes[1] = run_cjxl(
                        &args.cjxl,
                        &apng,
                        &rle_dir.join("atlas").join(format!("{gi}_anim_d0.jxl")),
                        &["-d", "0"],
                        7,
                    )?;
                    let _ = std::fs::remove_file(&apng);
                }
                Ok((sizes, anim_sizes))
            })
            .collect::<Result<Vec<_>>>()?;
        let atlas_s = t.elapsed().as_secs_f64();

        let mut atlas_sum = [0u64; 4];
        let mut anim_sum = [0u64; 2];
        for (a, an) in &group_res {
            for k in 0..4 {
                atlas_sum[k] += a[k];
            }
            anim_sum[0] += an[0];
            anim_sum[1] += an[1];
        }
        let grouped_ids: HashSet<u32> = atlas_groups
            .iter()
            .flat_map(|g| g.ids.iter().copied())
            .collect();
        let mut ps_sum = [0u64; 4];
        for id in &grouped_ids {
            for k in 0..4 {
                ps_sum[k] += per_sprite[id][k];
            }
        }
        writeln!(
            out,
            "atlas variants: {} of {} groups have >= 4 frames ({} frames, {:.0} s):",
            atlas_groups.len(),
            groups.len(),
            grouped_ids.len(),
            atlas_s,
        )?;
        writeln!(
            out,
            "  atlas       q90 {} | q80 {} | q70 {} | d0 {}",
            fmt_bytes(atlas_sum[0]),
            fmt_bytes(atlas_sum[1]),
            fmt_bytes(atlas_sum[2]),
            fmt_bytes(atlas_sum[3]),
        )?;
        writeln!(
            out,
            "  per-sprite  q90 {} | q80 {} | q70 {} | d0 {}   (same frames)",
            fmt_bytes(ps_sum[0]),
            fmt_bytes(ps_sum[1]),
            fmt_bytes(ps_sum[2]),
            fmt_bytes(ps_sum[3]),
        )?;
        if animated_ok {
            writeln!(
                out,
                "  animated    q80 {} | d0 {}   (APNG -> cjxl frame sequence, same frames)",
                fmt_bytes(anim_sum[0]),
                fmt_bytes(anim_sum[1]),
            )?;
        } else {
            writeln!(out, "  animated: cjxl rejected APNG input, skipped")?;
        }

        // Largest rhs files by selected raw payload.
        let mut per_stem: BTreeMap<&str, (usize, u64, u64, u64, u64)> = BTreeMap::new();
        for g in &groups {
            let stem = g.label.split(':').next().unwrap();
            let e = per_stem.entry(stem).or_default();
            for id in &g.ids {
                let s = by_id[id];
                e.0 += 1;
                e.1 += 10 + 2 * s.grid.len() as u64;
                e.2 += per_sprite[id][1];
                e.3 += per_sprite[id][2];
                e.4 += per_sprite[id][3];
            }
        }
        let mut stems: Vec<_> = per_stem.into_iter().collect();
        stems.sort_by_key(|(_, v)| std::cmp::Reverse(v.1));
        writeln!(
            out,
            "top rhs by selected raw payload: {:<24} {:>5} {:>10} {:>10} {:>10} {:>10}",
            "rhs", "n", "raw", "jxl-q80", "jxl-q70", "jxl-d0"
        )?;
        for (stem, (n, raw, q80, q70, d0)) in stems.iter().take(10) {
            writeln!(
                out,
                "                                 {:<24} {:>5} {:>10} {:>10} {:>10} {:>10}",
                stem,
                n,
                fmt_bytes(*raw),
                fmt_bytes(*q80),
                fmt_bytes(*q70),
                fmt_bytes(*d0),
            )?;
        }

        print!("{out}");
        report.push_str(&out);
        Ok(())
    }

    // ==================================================================
    // --pak mode: loading-art pictures
    // ==================================================================

    const DEFAULT_PAKS: &[&str] = &[
        "Data/Interface/Loading.pak",
        "2047/Data/Interface/Slideshow_in.pak",
    ];

    fn run_paks(args: &Args, report: &mut String) -> Result<()> {
        let rels: Vec<String> = if args.pak_files.is_empty() {
            DEFAULT_PAKS.iter().map(|s| (*s).to_string()).collect()
        } else {
            args.pak_files.clone()
        };
        for rel in &rels {
            let path = Path::new(&args.data_dir).join(rel);
            if !path.is_file() {
                eprintln!("# pak {} not found, skipped", path.display());
                continue;
            }
            probe_pak(args, rel, &path, report)?;
        }
        Ok(())
    }

    fn probe_pak(args: &Args, rel: &str, path: &Path, report: &mut String) -> Result<()> {
        let mut out = String::new();
        writeln!(out, "\n== pak {rel} ==")?;
        let mut file = SbFile::open(&path.to_string_lossy(), SB_FILE_READ)
            .map_err(|e| anyhow!("open {}: {e}", path.display()))?;
        let total = file.get_size();
        let mut pics: Vec<Picture> = Vec::new();
        while file.tell() < total {
            pics.push(
                Picture::load_sixteen_from_stream(&mut file)
                    .with_context(|| format!("pak picture {}", pics.len()))?,
            );
        }

        let stem = rel.replace(['/', ' '], "_");
        let pak_dir = args.out.join(format!("pak_{stem}"));
        std::fs::create_dir_all(pak_dir.join("worst"))?;

        let mut sels: Vec<Sel> = Vec::new();
        let mut raw565 = 0u64;
        let mut cat_raw: Vec<u8> = Vec::new();
        for (i, pic) in pics.iter().enumerate() {
            if pic.pixel_format != PixelFormat::Rgb16 {
                bail!(
                    "{rel}: picture {i} is {:?}, probe only handles Rgb16",
                    pic.pixel_format
                );
            }
            let n = pic.width as usize * pic.height as usize;
            raw565 += 2 * n as u64;
            cat_raw.extend_from_slice(&pic.data[..2 * n]);
            let pixels: Vec<u16> = (0..n)
                .map(|k| u16::from_le_bytes([pic.data[2 * k], pic.data[2 * k + 1]]))
                .collect();
            // The converter's interface path keys ONLY the transparent
            // color; shadow-key is an ordinary color in pictures.
            let classes = pixels
                .iter()
                .map(|&p| {
                    if p == TRANSPARENT_COLOR_16 {
                        CL_TRANS
                    } else {
                        CL_OPAQUE
                    }
                })
                .collect();
            sels.push(Sel {
                id: i as u32,
                width: pic.width,
                height: pic.height,
                grid: Vec::new(),
                pixels,
                classes,
            });
        }
        // Lossless baseline: what drop-bzip + outer shipping zstd gives.
        let z_raw = robin_assets::shipping_datadir::zstd_max_compress(&cat_raw)?.len() as u64;

        let t = Instant::now();
        let results: Vec<(u32, [u64; 4])> = sels
            .par_iter()
            .map(|s| -> Result<(u32, [u64; 4])> {
                let png_path = pak_dir.join(format!("{}.png", s.id));
                write_png(&png_path, s.width as u32, s.height as u32, &sprite_rgba(s))?;
                let mut sizes = [0u64; 4];
                for (k, (tag, qargs)) in QUALITY_SET.iter().enumerate() {
                    sizes[k] = run_cjxl(
                        &args.cjxl,
                        &png_path,
                        &pak_dir.join(format!("{}_{tag}.jxl", s.id)),
                        qargs,
                        9,
                    )?;
                }
                let _ = std::fs::remove_file(&png_path);
                Ok((s.id, sizes))
            })
            .collect::<Result<Vec<_>>>()?;
        let encode_s = t.elapsed().as_secs_f64();
        let sizes: HashMap<u32, [u64; 4]> = results.into_iter().collect();

        writeln!(
            out,
            "{} pictures, raw RGB565 {}, zstd-max lossless {} (cjxl e9 keyed RGBA like the \
             converter's interface path, {:.0} s)",
            pics.len(),
            fmt_bytes(raw565),
            fmt_bytes(z_raw),
            encode_s,
        )?;
        writeln!(
            out,
            "{:>4} {:>9} | {:>9} {:>9} {:>9} {:>9} | {:>8} {:>8} {:>8}",
            "pic", "dims", "d0", "q90", "q80", "q70", "psnr90", "psnr80", "psnr70",
        )?;
        let mut tot = [0u64; 4];
        let mut worst: Option<(f64, u32)> = None;
        let mut aggs = [QualityStats::default(); 3];
        for s in &sels {
            let v = sizes[&s.id];
            for k in 0..4 {
                tot[k] += v[k];
            }
            let mut psnrs = [0f64; 3];
            for (qi, (tag, _)) in QUALITY_SET.iter().take(3).enumerate() {
                let bytes = std::fs::read(pak_dir.join(format!("{}_{tag}.jxl", s.id)))?;
                let (w, h, rgba) = decode_jxl_rgba(&bytes)?;
                if (w, h) != (s.width as usize, s.height as usize) {
                    bail!("pak {rel}: decode size mismatch on picture {}", s.id);
                }
                let q = score_decoded(s, &rgba);
                psnrs[qi] = q.psnr();
                aggs[qi].add(&q);
                if *tag == "q80" && worst.is_none_or(|(p, _)| q.psnr() < p) {
                    worst = Some((q.psnr(), s.id));
                }
            }
            writeln!(
                out,
                "{:>4} {:>9} | {:>9} {:>9} {:>9} {:>9} | {:>6.1}dB {:>6.1}dB {:>6.1}dB",
                s.id,
                format!("{}x{}", s.width, s.height),
                fmt_bytes(v[3]),
                fmt_bytes(v[0]),
                fmt_bytes(v[1]),
                fmt_bytes(v[2]),
                psnrs[0],
                psnrs[1],
                psnrs[2],
            )?;
        }
        writeln!(
            out,
            "TOTAL d0 {} | q90 {} | q80 {} | q70 {}  (vs raw {}, zstd-lossless {})",
            fmt_bytes(tot[3]),
            fmt_bytes(tot[0]),
            fmt_bytes(tot[1]),
            fmt_bytes(tot[2]),
            fmt_bytes(raw565),
            fmt_bytes(z_raw),
        )?;
        for (qi, (tag, _)) in QUALITY_SET.iter().take(3).enumerate() {
            writeln!(
                out,
                "{tag}: PSNR {:.1} dB, 565-exact {:.1}%, key-collisions {} / {} px",
                aggs[qi].psnr(),
                100.0 * aggs[qi].exact565 as f64 / aggs[qi].opaque_px.max(1) as f64,
                aggs[qi].key_collisions,
                aggs[qi].opaque_px,
            )?;
        }
        if let Some((psnr, id)) = worst {
            let s = sels.iter().find(|s| s.id == id).unwrap();
            for tag in ["q80", "q70"] {
                let bytes = std::fs::read(pak_dir.join(format!("{id}_{tag}.jxl")))?;
                let (_, _, rgba) = decode_jxl_rgba(&bytes)?;
                let q = score_decoded(s, &rgba);
                let label = if tag == "q80" { psnr } else { q.psnr() };
                dump_side_by_side(
                    &pak_dir
                        .join("worst")
                        .join(format!("{id}_{tag}_psnr{label:.1}.png")),
                    s,
                    &rgba,
                )?;
            }
        }
        print!("{out}");
        report.push_str(&out);
        Ok(())
    }
}
