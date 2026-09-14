//! Web image codec tuning harness (RLE-JXL atlases, keyed interface
//! pictures, terrain maps): corpus extraction, serial decode timing and
//! quality scoring against the exact encoder input.
//!
//! ```text
//! # native: pull a mission's shipped JXL blobs out of a shipping datadir
//! cargo run --release -p robin_assets --example rle_jxl_tuning_bench -- \
//!     extract <Data> <mission> <out-dir>
//! # serial decode timing of every *.jxl in a directory (native or WASI)
//! ... -- time <dir> [repeats]
//! # quality: every <src>/<name>.png (the exact cjxl input) against
//! # <cand>/<name>.jxl (decoded here with jxl-rs) or <cand>/<name>.png
//! # (decoded elsewhere, e.g. avifdec or a browser dump)
//! ... -- score <src-dir> <cand-dir>
//! ```
//!
//! WASI build (serial wasm timing under node, same codegen flags as the web
//! build):
//!
//! ```text
//! CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS='-C target-feature=+simd128 -C passes=loop-vectorize,slp-vectorizer,instcombine<no-verify-fixpoint>,simplifycfg' \
//!   cargo build --profile wasm-release --target wasm32-wasip1 --no-default-features \
//!   -p robin_assets --example rle_jxl_tuning_bench
//! node scripts/wasi_run.mjs target/wasm32-wasip1/wasm-release/examples/rle_jxl_tuning_bench.wasm time <dir> 5
//! ```
//!
//! Quality metric: class (alpha marker) mismatches must be zero; colour is
//! scored over opaque pixels after the runtime's RGB565 requantization
//! (`quant565`), exactly like the converter's per-sprite PSNR gate.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Instant;

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Decode to interleaved u8 with the image's own channel count (3 or 4).
fn decode_jxl(bytes: &[u8]) -> Result<(usize, usize, usize, Vec<u8>)> {
    use jxl::api::{
        JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
        JxlPixelFormat, ProcessingResult, states,
    };
    let mut input: &[u8] = bytes;
    let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());
    let mut dec = match dec.process(&mut input, None) {
        Ok(ProcessingResult::Complete { result }) => result,
        other => bail!("jxl header: {:?}", other.err()),
    };
    let (w, h) = dec.basic_info().size;
    let has_alpha = !dec.basic_info().extra_channels.is_empty();
    let channels = if has_alpha { 4 } else { 3 };
    dec.set_pixel_format(JxlPixelFormat {
        color_type: if has_alpha {
            JxlColorType::Rgba
        } else {
            JxlColorType::Rgb
        },
        color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
        extra_channel_format: if has_alpha {
            vec![None; dec.basic_info().extra_channels.len()]
        } else {
            vec![]
        },
    })
    .map_err(|e| anyhow::anyhow!("pixel format: {e:?}"))?;
    let dec = match dec.process(&mut input, None) {
        Ok(ProcessingResult::Complete { result }) => result,
        other => bail!("jxl frame header: {:?}", other.err()),
    };
    let mut pixels = vec![0u8; w * h * channels];
    let mut bufs = vec![JxlOutputBuffer::new(&mut pixels, h, w * channels)];
    match dec.process(&mut input, &mut bufs, None) {
        Ok(ProcessingResult::Complete { .. }) => {}
        other => bail!("jxl frame: {:?}", other.err()),
    }
    drop(bufs);
    Ok((w, h, channels, pixels))
}

fn list(dir: &Path, ext: &str) -> Result<Vec<std::path::PathBuf>> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    out.sort();
    Ok(out)
}

fn time(dir: &Path, repeats: usize) -> Result<()> {
    let files = list(dir, "jxl")?;
    let blobs: Vec<Vec<u8>> = files
        .iter()
        .map(std::fs::read)
        .collect::<std::io::Result<_>>()?;
    let bytes: usize = blobs.iter().map(Vec::len).sum();
    let mut best = f64::INFINITY;
    let mut out_hash = 0u64;
    let mut decoded = 0usize;
    for _ in 0..repeats {
        let start = Instant::now();
        let mut h = 0u64;
        decoded = 0;
        for blob in &blobs {
            let (_, _, _, px) = decode_jxl(blob)?;
            decoded += px.len();
            h = h.rotate_left(7) ^ fnv(&px);
        }
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
        out_hash = h;
    }
    println!(
        "{{\"dir\":{:?},\"files\":{},\"bytes\":{bytes},\"decoded_bytes\":{decoded},\"serial_best_ms\":{best:.1},\"hash\":\"{out_hash:016x}\"}}",
        dir.display().to_string(),
        blobs.len()
    );
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let mut best_par = f64::INFINITY;
        for _ in 0..repeats {
            let start = Instant::now();
            blobs
                .par_iter()
                .map(|b| decode_jxl(b).map(|_| ()))
                .collect::<Result<Vec<_>>>()?;
            best_par = best_par.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!("  atlas-parallel best: {best_par:.1} ms");
    }
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), feature = "engine-adapters"))]
mod native {
    use super::*;
    use robin_assets::rle_jxl::{ALPHA_OPAQUE, alpha_to_class, quant565};
    use robin_assets::shipping_datadir::{
        ShippingDatadir, ShippingMission, decode_mission_compressed,
    };

    pub fn extract(root: &Path, mission: &str, out: &Path) -> Result<()> {
        let dd = ShippingDatadir::from_compressed_bytes(&std::fs::read(root.join("datadir.bin"))?)?;
        let reference = dd.missions.get(mission).context("mission absent")?;
        let mut merged = ShippingMission::default();
        for part in &reference.files {
            merged.merge_part(decode_mission_compressed(&std::fs::read(root.join(part))?)?)?;
        }
        let bank = merged
            .payload
            .sprite_bank
            .as_ref()
            .context("no sprite bank")?;
        std::fs::create_dir_all(out.join("atlas"))?;
        std::fs::create_dir_all(out.join("terrain"))?;
        let mut n = 0;
        for chunk in &bank.rle_jxl_chunks {
            for blob in &chunk.jxl_blobs {
                std::fs::write(
                    out.join("atlas").join(format!("{:016x}.jxl", fnv(blob))),
                    blob,
                )?;
                n += 1;
            }
        }
        let mut t = 0;
        for (name, bytes) in &merged.payload.raw {
            let lower = name.to_ascii_lowercase();
            let is_terrain = lower.ends_with(".map") || lower.ends_with(".min");
            let is_jxl = bytes.starts_with(&[0xff, 0x0a])
                || bytes.starts_with(&[0, 0, 0, 0x0c, b'J', b'X', b'L', b' ']);
            if is_terrain && is_jxl {
                std::fs::write(
                    out.join("terrain").join(format!("{:016x}.jxl", fnv(bytes))),
                    bytes,
                )?;
                println!("terrain {name} {} bytes", bytes.len());
                t += 1;
            }
        }
        println!("extracted {n} atlas blobs, {t} terrain blobs");
        Ok(())
    }

    fn write_png(path: &Path, w: usize, h: usize, channels: usize, px: &[u8]) -> Result<()> {
        let mut enc = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            w as u32,
            h as u32,
        );
        enc.set_color(match channels {
            3 => png::ColorType::Rgb,
            4 => png::ColorType::Rgba,
            other => bail!("unsupported channel count {other}"),
        });
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header()?;
        writer.write_image_data(px)?;
        writer.finish()?;
        Ok(())
    }

    /// Sixteen `.map` -> RGB888 PNG, the exact pixels `convert_datadir`'s
    /// `picture_to_rgb888` hands cjxl for terrain (RGB565 bit replication).
    pub fn map2png(map: &Path, out: &Path) -> Result<()> {
        use robin_assets::picture::{Picture, PixelFormat};
        let mut file = robin_data_io::sbfile::SbFile::open(&map.to_string_lossy())
            .map_err(|e| anyhow::anyhow!("open {}: {e}", map.display()))?;
        let pic = Picture::load_sixteen_from_stream(&mut file)
            .with_context(|| format!("decode {}", map.display()))?;
        anyhow::ensure!(
            pic.pixel_format == PixelFormat::Rgb16,
            "{}: expected an Rgb16 terrain picture, got {:?}",
            map.display(),
            pic.pixel_format
        );
        let (w, h) = (usize::from(pic.width), usize::from(pic.height));
        let pitch = if pic.pitch == 0 {
            w * 2
        } else {
            usize::from(pic.pitch)
        };
        anyhow::ensure!(pitch >= w * 2, "pitch shorter than a row");
        anyhow::ensure!(
            h == 0 || pic.data.len() >= (h - 1) * pitch + w * 2,
            "picture data truncated"
        );
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let o = y * pitch + x * 2;
                let word = u16::from_le_bytes([pic.data[o], pic.data[o + 1]]);
                rgb.extend(robin_assets::rle_jxl::expand565(word));
            }
        }
        write_png(out, w, h, 3, &rgb)?;
        println!("{} {w}x{h}", out.display());
        Ok(())
    }

    /// Decode with jxl-rs (the runtime decoder) and write the result as PNG.
    pub fn jxl2png(jxl: &Path, out: &Path) -> Result<()> {
        let (w, h, channels, px) = decode_jxl(&std::fs::read(jxl)?)?;
        write_png(out, w, h, channels, &px)
    }

    fn read_png(path: &Path) -> Result<(usize, usize, usize, Vec<u8>)> {
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size().context("png size")?];
        let info = reader.next_frame(&mut buf)?;
        anyhow::ensure!(info.bit_depth == png::BitDepth::Eight, "8-bit png only");
        let channels = match info.color_type {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => bail!("{}: unsupported png color {other:?}", path.display()),
        };
        buf.truncate(info.width as usize * info.height as usize * channels);
        Ok((info.width as usize, info.height as usize, channels, buf))
    }

    pub fn score(src_dir: &Path, cand_dir: &Path) -> Result<()> {
        let (mut sse, mut opaque_px, mut bytes, mut files) = (0f64, 0u64, 0usize, 0usize);
        let (mut alpha_bad, mut class_bad, mut max_err) = (0u64, 0u64, 0i32);
        let mut worst = (f64::INFINITY, String::new());
        for src in list(src_dir, "png")? {
            let stem = src.file_stem().unwrap().to_string_lossy().to_string();
            let (sw, sh, sc, s) = read_png(&src)?;
            let jxl = cand_dir.join(format!("{stem}.jxl"));
            let avif = cand_dir.join(format!("{stem}.avif"));
            let dec_png = cand_dir.join(format!("{stem}.png"));
            let raw = cand_dir.join(format!("{stem}.rgba"));
            let (cw, ch, cc, c) = if raw.exists() {
                // Browser dump: straight RGBA8 at the source dimensions.
                let px = std::fs::read(&raw)?;
                anyhow::ensure!(px.len() == sw * sh * 4, "{stem}: raw dump size");
                (sw, sh, 4, px)
            } else if jxl.exists() {
                let b = std::fs::read(&jxl)?;
                bytes += b.len();
                decode_jxl(&b)?
            } else if dec_png.exists() {
                if avif.exists() {
                    bytes += std::fs::metadata(&avif)?.len() as usize;
                }
                read_png(&dec_png)?
            } else {
                bail!("no candidate for {stem}");
            };
            anyhow::ensure!((sw, sh) == (cw, ch), "{stem}: dims differ");
            files += 1;
            let (mut f_sse, mut f_n) = (0f64, 0u64);
            for i in 0..sw * sh {
                let sp = &s[i * sc..i * sc + sc];
                let cp = &c[i * cc..i * cc + cc];
                let visible = if sc == 4 {
                    let ca = if cc == 4 { cp[3] } else { 255 };
                    if ca != sp[3] {
                        alpha_bad += 1;
                    }
                    let sclass = alpha_to_class(sp[3])?;
                    if alpha_to_class(ca).ok() != Some(sclass) {
                        class_bad += 1;
                    }
                    sp[3] == ALPHA_OPAQUE
                } else {
                    true
                };
                if !visible {
                    continue;
                }
                let a = quant565(sp[0], sp[1], sp[2]);
                let b = quant565(cp[0], cp[1], cp[2]);
                let ea = robin_assets::rle_jxl::expand565(a);
                let eb = robin_assets::rle_jxl::expand565(b);
                for k in 0..3 {
                    let d = i32::from(ea[k]) - i32::from(eb[k]);
                    max_err = max_err.max(d.abs());
                    f_sse += f64::from(d * d);
                }
                f_n += 3;
            }
            let f_psnr = 10.0 * (255.0f64 * 255.0 / (f_sse / f_n.max(1) as f64)).log10();
            if f_psnr < worst.0 {
                worst = (f_psnr, stem.clone());
            }
            sse += f_sse;
            opaque_px += f_n;
        }
        let psnr = 10.0 * (255.0f64 * 255.0 / (sse / opaque_px.max(1) as f64)).log10();
        println!(
            "{{\"cand\":{:?},\"files\":{files},\"bytes\":{bytes},\"psnr565\":{psnr:.2},\"worst_file_psnr\":{:.2},\"worst\":{:?},\"max_abs_err\":{max_err},\"alpha_mismatch\":{alpha_bad},\"class_mismatch\":{class_bad}}}",
            cand_dir.display().to_string(),
            worst.0,
            worst.1
        );
        Ok(())
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("time") => time(
            Path::new(&args[2]),
            args.get(3).map_or(Ok(3), |r| r.parse())?,
        ),
        #[cfg(all(not(target_arch = "wasm32"), feature = "engine-adapters"))]
        Some("extract") => native::extract(Path::new(&args[2]), &args[3], Path::new(&args[4])),
        #[cfg(all(not(target_arch = "wasm32"), feature = "engine-adapters"))]
        Some("score") => native::score(Path::new(&args[2]), Path::new(&args[3])),
        #[cfg(all(not(target_arch = "wasm32"), feature = "engine-adapters"))]
        Some("map2png") => native::map2png(Path::new(&args[2]), Path::new(&args[3])),
        #[cfg(all(not(target_arch = "wasm32"), feature = "engine-adapters"))]
        Some("jxl2png") => native::jxl2png(Path::new(&args[2]), Path::new(&args[3])),
        _ => bail!(
            "usage: extract <Data> <mission> <out> | time <dir> [repeats] | score <src> <cand> \
             | map2png <map> <out.png> | jxl2png <in.jxl> <out.png>"
        ),
    }
}
