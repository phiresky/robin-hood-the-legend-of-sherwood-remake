//! Custom sprite manifests and portable encoding, independent of game sessions.
//! Disposable cache persistence and live installation remain client responsibilities.
pub mod family;
pub mod shipping;
use crate::frame_holder as assets_frame_holder;
pub use family::encode_custom_sprite_family;
use robin_data_io::sbfile as engine_sbfile;
use robin_engine::coordinates::{SpriteAnchor, SpriteFrameOffset, SpriteLocalPoint, SpriteSize};
use robin_engine::sprite_script::{NONANIMATION_END, SpriteInfo, SpriteScript, UNMAPPED};
pub use shipping::encode_custom_sprite_dir;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HackableRhsManifest {
    pub pixel_format: HackableRhsPixelFormat,
    pub profiles: Vec<HackableRhsProfile>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HackableRhsPixelFormat {
    Rgba,
    LegacyColorKeys,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HackableRhsProfile {
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub center_x: f32,
    pub center_y: f32,
    pub rows: Vec<HackableRhsRow>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HackableRhsRow {
    pub action_id: u16,
    pub action_done: u16,
    pub average_speed: f32,
    #[serde(default, rename = "direction")]
    pub _direction: u16,
    pub hotspot_x: f32,
    pub hotspot_y: f32,
    pub path: String,
    pub frames: Vec<HackableRhsFrame>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HackableRhsFrame {
    pub file: String,
    pub delay: u16,
    pub distance: u16,
    pub offset_x: f32,
    pub offset_y: f32,
    pub sound_id: u16,
}

pub const HACKABLE_RHS_CACHE_VERSION: u32 = 2;

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct HackableRhsCache {
    pub version: u32,
    pub manifest_hash: [u8; 32],
    pub sources: Vec<HackableRhsCacheSource>,
    pub frames: Vec<assets_frame_holder::RuntimeSprite>,
    pub profiles: Vec<HackableRhsCacheProfile>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct HackableRhsCacheSource {
    pub relative_path: String,
    pub len: u64,
    pub modified_secs: u64,
    pub modified_nanos: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct HackableRhsCacheProfile {
    pub name: String,
    pub info: SpriteInfo,
}

pub fn decode_png_rgba_bytes(bytes: &[u8], source: &str) -> Result<(u16, u16, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("decode {source}: {e}"))?;
    let width = u16::try_from(reader.info().width)
        .map_err(|_| format!("sprite width exceeds u16 for {source}"))?;
    let height = u16::try_from(reader.info().height)
        .map_err(|_| format!("sprite height exceeds u16 for {source}"))?;
    let rgba_len = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("sprite RGBA output size overflows for {source}"))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("unknown PNG output size for {source}"))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("read frame {source}: {e}"))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(rgba_len);
            for px in buf.as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(rgba_len);
            for &value in &buf {
                out.extend_from_slice(&[value, value, value, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(rgba_len);
            for px in buf.as_chunks::<2>().0 {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        other => {
            return Err(format!(
                "PNG decoder did not expand color type {other:?} for {source}"
            ));
        }
    };
    Ok((width, height, rgba))
}

pub fn hackable_manifest_hash(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

pub fn hackable_source_stamp(
    root: &std::path::Path,
    relative_path: &str,
) -> Result<HackableRhsCacheSource, String> {
    let requested = root.join(relative_path);
    let path = engine_sbfile::resolve_case_insensitive(&requested).unwrap_or(requested);
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("stat hackable sprite {}: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("read mtime for {}: {error}", path.display()))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("invalid mtime for {}: {error}", path.display()))?;
    Ok(HackableRhsCacheSource {
        relative_path: relative_path.to_owned(),
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

pub fn hackable_animation_conversion(scripts: &[SpriteScript]) -> Vec<u16> {
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    for (row_index, script) in scripts.iter().enumerate() {
        if let Some(slot) = conversion.get_mut(script.action_id as usize)
            && *slot == UNMAPPED
        {
            *slot = row_index as u16;
        }
    }

    // Minimal hackable characters may only provide idle and walking loops.
    // Install fallbacks only after every authored action has claimed its own
    // slot, so a real run row always wins over the walking fallback.
    for (source, aliases) in [
        (3usize, &[0usize, 1, 2, 4, 8][..]),
        (6, &[5, 7, 9, 10, 11, 12][..]),
    ] {
        let source_row = conversion[source];
        if source_row == UNMAPPED {
            continue;
        }
        for &alias in aliases {
            if conversion[alias] == UNMAPPED {
                conversion[alias] = source_row;
            }
        }
    }
    conversion
}

/// Import a loose PNG sprite directory without installing it in a game session.
pub fn build_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
    manifest: HackableRhsManifest,
) -> anyhow::Result<HackableRhsCache> {
    build_hackable_cache_with_reader(manifest_hash, manifest, |relative, legacy| {
        use anyhow::Context as _;
        let path = root.join(relative);
        let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
        let (width, height, rgba) = decode_png_rgba_bytes(&bytes, &path.display().to_string())
            .map_err(anyhow::Error::msg)?;
        let source = hackable_source_stamp(root, relative).map_err(anyhow::Error::msg)?;
        Ok((
            assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                width, height, &rgba, legacy,
            ),
            Some(source),
        ))
    })
}
pub fn build_hackable_cache_with_reader<E>(
    manifest_hash: [u8; 32],
    manifest: HackableRhsManifest,
    mut read_frame: impl FnMut(
        &str,
        bool,
    ) -> Result<
        (
            assets_frame_holder::RuntimeSprite,
            Option<HackableRhsCacheSource>,
        ),
        E,
    >,
) -> Result<HackableRhsCache, E> {
    let mut frames = Vec::new();
    let mut sources = Vec::new();
    let mut local_frames = std::collections::HashMap::<String, u32>::new();
    let mut profiles = Vec::with_capacity(manifest.profiles.len());
    let legacy_color_keys = matches!(
        manifest.pixel_format,
        HackableRhsPixelFormat::LegacyColorKeys
    );

    for profile in manifest.profiles {
        let mut scripts = Vec::with_capacity(profile.rows.len());
        for row in profile.rows {
            let mut script = SpriteScript {
                action_id: row.action_id,
                action_done: row.action_done,
                average_speed: row.average_speed,
                hotspot: SpriteLocalPoint::new(row.hotspot_x, row.hotspot_y),
                ..SpriteScript::default()
            };
            for frame in row.frames {
                let relative_path = std::path::Path::new(&row.path)
                    .join(&frame.file)
                    .to_string_lossy()
                    .into_owned();
                let local_id = if let Some(local_id) = local_frames.get(&relative_path) {
                    *local_id
                } else {
                    let (sprite, source) = read_frame(&relative_path, legacy_color_keys)?;
                    let local_id = frames.len() as u32;
                    frames.push(sprite);
                    sources.extend(source);
                    local_frames.insert(relative_path, local_id);
                    local_id
                };
                script.frame_ids.push(local_id);
                script.delays.push(frame.delay);
                script.distances.push(frame.distance);
                script
                    .offsets
                    .push(SpriteFrameOffset::new(frame.offset_x, frame.offset_y));
                script.sound_ids.push(frame.sound_id);
                script.sum_distance = script.sum_distance.saturating_add(frame.distance);
            }
            scripts.push(script);
        }
        let conversion = hackable_animation_conversion(&scripts);
        profiles.push(HackableRhsCacheProfile {
            name: profile.name,
            info: SpriteInfo {
                scripts: std::sync::Arc::new(scripts),
                conversion: std::sync::Arc::new(conversion),
                size: SpriteSize::new(profile.width, profile.height),
                center: SpriteAnchor::new(profile.center_x, profile.center_y),
            },
        });
    }

    Ok(HackableRhsCache {
        version: HACKABLE_RHS_CACHE_VERSION,
        manifest_hash,
        sources,
        frames,
        profiles,
    })
}

pub fn validate_cache_frames(filename: &str, cache: &HackableRhsCache) -> Result<(), String> {
    for profile in &cache.profiles {
        for script in profile.info.scripts.iter() {
            for frame_id in &script.frame_ids {
                if *frame_id as usize >= cache.frames.len() {
                    return Err(format!(
                        "{filename}: sprite profile {} references missing local frame {frame_id}",
                        profile.name
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_decoder_rejects_oversized_headers_before_pixel_decoding() {
        for (width, height, expected) in [
            (65_536, 1, "sprite width exceeds u16"),
            (1, 65_536, "sprite height exceeds u16"),
        ] {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_chunk(png::chunk::IDAT, &[]).unwrap();
            drop(writer);
            let error = decode_png_rgba_bytes(&bytes, "oversized test").unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn png_decoder_preserves_supported_color_and_depth_conversions() {
        for (color, depth, source, expected) in [
            (
                png::ColorType::Rgb,
                png::BitDepth::Eight,
                vec![1, 2, 3, 4, 5, 6],
                vec![1, 2, 3, 255, 4, 5, 6, 255],
            ),
            (
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                vec![1, 2, 3, 0, 4, 5, 6, 127],
                vec![1, 2, 3, 0, 4, 5, 6, 127],
            ),
            (
                png::ColorType::Grayscale,
                png::BitDepth::Eight,
                vec![17, 231],
                vec![17, 17, 17, 255, 231, 231, 231, 255],
            ),
            (
                png::ColorType::GrayscaleAlpha,
                png::BitDepth::Eight,
                vec![17, 0, 231, 127],
                vec![17, 17, 17, 0, 231, 231, 231, 127],
            ),
            (
                png::ColorType::Rgb,
                png::BitDepth::Sixteen,
                vec![1, 255, 2, 255, 3, 255, 4, 255, 5, 255, 6, 255],
                vec![1, 2, 3, 255, 4, 5, 6, 255],
            ),
            (
                png::ColorType::Grayscale,
                png::BitDepth::Sixteen,
                vec![17, 255, 231, 255],
                vec![17, 17, 17, 255, 231, 231, 231, 255],
            ),
        ] {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&source).unwrap();
            writer.finish().unwrap();
            assert_eq!(
                decode_png_rgba_bytes(&bytes, "color test").unwrap(),
                (2, 1, expected)
            );
        }
    }

    #[test]
    fn png_decoder_expands_indexed_pixels_and_palette_transparency() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![0, 255, 0, 0, 0, 255]);
            encoder.set_trns(vec![0, 127]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }

        let (width, height, rgba) = decode_png_rgba_bytes(&encoded, "indexed test").unwrap();

        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, [0, 255, 0, 0, 0, 0, 255, 127]);
    }

    #[test]
    fn hackable_animation_fallbacks_do_not_override_authored_actions() {
        let script = |action_id| SpriteScript {
            action_id,
            ..SpriteScript::default()
        };
        let conversion = hackable_animation_conversion(&[script(3), script(6), script(10)]);

        assert_eq!(conversion[9], 1, "missing transition reuses walking");
        assert_eq!(conversion[10], 2, "authored running row must win");

        let minimal = hackable_animation_conversion(&[script(3), script(6)]);
        assert_eq!(minimal[10], 1, "minimal sprites may reuse walking");
    }
}
