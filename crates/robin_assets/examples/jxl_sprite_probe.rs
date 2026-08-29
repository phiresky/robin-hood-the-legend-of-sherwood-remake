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
//! Usage:
//!
//! ```text
//! cargo build --release --example jxl_sprite_probe
//! cargo run --release --example jxl_sprite_probe -- \
//!     --data-dir datadirs/fullgame_linux \
//!     --out tmp/jxl_sprite_probe \
//!     --cjxl /path/to/cjxl \
//!     [--rhs "Characters/Knight01.rhs"]... [--min-dim 20] [--limit 0]
//! ```

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(not(target_arch = "wasm32"))]
mod probe {
    use std::collections::BTreeSet;
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use anyhow::{Context, Result, anyhow, bail};
    use rayon::prelude::*;
    use robin_assets::frame_holder::{
        FrameHolder, SHADOW_KEY, TRANSPARENT_COLOR_16, UNMAPPED_DICT,
    };
    use robin_assets::shipping_datadir::derive_chunk_self_refs;
    use robin_assets::sprite_codec::{self, SpriteGrid};
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
                other => bail!("unknown argument {other}"),
            }
        }
        if args.rhs.is_empty() {
            args.rhs = DEFAULT_RHS.iter().map(|s| (*s).to_owned()).collect();
        }
        Ok(args)
    }

    /// One selected VQ sprite, fully expanded.
    struct Sel {
        id: u32,
        width: u16,
        height: u16,
        /// VQ tile-index grid, `(width/4) x height`.
        grid: Vec<u16>,
        /// Decoded RGB565 pixels (Day dictionary), `width x height`.
        pixels: Vec<u16>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Class {
        Transparent,
        Shadow,
        Opaque,
    }

    fn classify(px: u16) -> Class {
        match px {
            TRANSPARENT_COLOR_16 => Class::Transparent,
            SHADOW_KEY => Class::Shadow,
            _ => Class::Opaque,
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

    /// RGBA export: opaque -> expanded 565 + alpha 255; transparent AND
    /// shadow -> (0,0,0,0). Keys ship in the lossless class map instead.
    fn sprite_rgba(s: &Sel) -> Vec<u8> {
        let mut rgba = vec![0u8; s.pixels.len() * 4];
        for (i, &px) in s.pixels.iter().enumerate() {
            if classify(px) == Class::Opaque {
                let [r, g, b] = expand565(px);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        rgba
    }

    /// 2-bit class map, packed 4 px/byte row-major.
    fn class_map_bits(s: &Sel) -> Vec<u8> {
        let mut out = vec![0u8; s.pixels.len().div_ceil(4)];
        for (i, &px) in s.pixels.iter().enumerate() {
            let code = match classify(px) {
                Class::Transparent => 0u8,
                Class::Shadow => 1,
                Class::Opaque => 2,
            };
            out[i / 4] |= code << ((i % 4) * 2);
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

    fn run_cjxl(cjxl: &str, input: &Path, output: &Path, quality: &[&str]) -> Result<u64> {
        let out = std::process::Command::new(cjxl)
            .arg(input)
            .arg(output)
            .args(quality)
            .args(["-e", "7", "--quiet"])
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
        if dec_with_image.basic_info().extra_channels.is_empty() {
            bail!("jxl: expected an alpha channel");
        }
        dec_with_image.set_pixel_format(JxlPixelFormat {
            color_type: JxlColorType::Rgba,
            color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
            extra_channel_format: vec![None],
        });
        let dec_with_frame = match dec_with_image.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame header"),
            Err(e) => bail!("jxl: frame header error: {e:?}"),
        };
        let stride = w * 4;
        let mut rgba = vec![0u8; stride * h];
        let mut bufs = vec![JxlOutputBuffer::new(&mut rgba, h, stride)];
        match dec_with_frame.process(&mut input, &mut bufs, None) {
            Ok(ProcessingResult::Complete { .. }) => {}
            Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame"),
            Err(e) => bail!("jxl: frame error: {e:?}"),
        }
        drop(bufs);
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
            if classify(px) != Class::Opaque {
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
                let (orig, dec) = match classify(px) {
                    Class::Transparent => ([96, 96, 96, 255], [96, 96, 96, 255]),
                    Class::Shadow => ([0, 0, 255, 255], [0, 0, 255, 255]),
                    Class::Opaque => {
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
            run_cjxl(&args.cjxl, &probe_png, &probe_jxl, &["-q", "90"])
                .context("cjxl sanity check failed — pass --cjxl <path>")?;
            let _ = std::fs::remove_file(probe_png);
            let _ = std::fs::remove_file(probe_jxl);
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
            selected.push(Sel {
                id: *id,
                width: w,
                height: h,
                grid: grid.clone(),
                pixels,
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
                )?;
                let q80 = run_cjxl(
                    &args.cjxl,
                    &png_path,
                    &char_dir.join("q80").join(format!("{}.jxl", s.id)),
                    &["-q", "80"],
                )?;
                let d0 = run_cjxl(
                    &args.cjxl,
                    &png_path,
                    &char_dir.join("d0").join(format!("{}.jxl", s.id)),
                    &["-d", "0"],
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
        let atlas_png = char_dir.join("atlas.png");
        write_png(&atlas_png, aw as u32, ah as u32, &rgba)?;
        let a90 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_q90.jxl"),
            &["-q", "90"],
        )?;
        let a80 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_q80.jxl"),
            &["-q", "80"],
        )?;
        let a0 = run_cjxl(
            &args.cjxl,
            &atlas_png,
            &char_dir.join("atlas_d0.jxl"),
            &["-d", "0"],
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
}
