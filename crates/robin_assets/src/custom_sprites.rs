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
    /// Omitted directions retain per-action encounter order. Explicit directions
    /// must cover a contiguous range starting at zero; manifest order is immaterial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<u16>,
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

pub const HACKABLE_RHS_CACHE_VERSION: u32 = 3;

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
    let pixels = crate::packed_sprite::pixel_count(width.into(), height.into())
        .map_err(|error| format!("{source}: {error}"))?;
    let rgba_len = pixels * 4;
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
    build_hackable_cache_with_reader(
        manifest_hash,
        manifest,
        |relative, legacy| {
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
        },
        anyhow::Error::msg,
    )
}
pub fn build_hackable_cache_with_reader<E>(
    manifest_hash: [u8; 32],
    mut manifest: HackableRhsManifest,
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
    invalid: impl Fn(String) -> E,
) -> Result<HackableRhsCache, E> {
    normalize_manifest_rows(&mut manifest).map_err(&invalid)?;
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

    let cache = HackableRhsCache {
        version: HACKABLE_RHS_CACHE_VERSION,
        manifest_hash,
        sources,
        frames,
        profiles,
    };
    validate_cache_frames("custom sprites", &cache).map_err(invalid)?;
    Ok(cache)
}

fn normalize_manifest_rows(manifest: &mut HackableRhsManifest) -> Result<(), String> {
    // TODO: declare directional coverage explicitly in the authored format.
    // Single-row actions are supported content, and original exports can have
    // 32 rows for one action; neither should silently acquire invented frames.
    for profile in &mut manifest.profiles {
        let mut order = std::collections::HashMap::new();
        let mut counts = std::collections::HashMap::<u16, u16>::new();
        for row in &mut profile.rows {
            let next_order = order.len();
            order.entry(row.action_id).or_insert(next_order);
            let count = counts.entry(row.action_id).or_default();
            row.direction.get_or_insert(*count);
            *count = count
                .checked_add(1)
                .ok_or_else(|| format!("{}: too many direction rows", profile.name))?;
        }
        profile
            .rows
            .sort_by_key(|row| (order[&row.action_id], row.direction));
        counts.clear();
        for row in &profile.rows {
            let expected = counts.entry(row.action_id).or_default();
            if row.direction != Some(*expected) {
                return Err(format!(
                    "{}: action {} has duplicate or missing direction: expected {}, got {:?}",
                    profile.name, row.action_id, expected, row.direction
                ));
            }
            *expected += 1;
        }
        for (action_id, count) in counts {
            if count < 16 {
                tracing::warn!(profile = %profile.name, action_id, authored_directions = count,
                    "Custom action has partial directional coverage; callers must restrict directions to its authored rows");
            }
        }
    }
    Ok(())
}

pub fn validate_cache_frames(filename: &str, cache: &HackableRhsCache) -> Result<(), String> {
    for (index, frame) in cache.frames.iter().enumerate() {
        frame
            .validate()
            .map_err(|error| format!("{filename}: frame {index}: {error}"))?;
    }
    validate_cache_metadata(filename, cache, cache.frames.len())
}

// VQ families need the same metadata check before remapping local IDs and
// using action layouts to decode pixel groups. Their frames are still packed.
fn validate_cache_metadata(
    filename: &str,
    cache: &HackableRhsCache,
    frame_count: usize,
) -> Result<(), String> {
    if !matches!(cache.version, 2 | HACKABLE_RHS_CACHE_VERSION) {
        return Err(format!(
            "{filename}: unsupported custom sprite metadata version {}",
            cache.version
        ));
    }
    let mut names = std::collections::HashSet::new();
    for profile in &cache.profiles {
        let fail = |detail: &str| format!("{filename}: sprite profile {}: {detail}", profile.name);
        if profile.name.is_empty() || !names.insert(&profile.name) {
            return Err(fail("empty or duplicate profile name"));
        }
        let info = &profile.info;
        if ![info.size.x, info.size.y, info.center.x, info.center.y]
            .iter()
            .all(|value| value.is_finite())
            || info.size.x <= 0.0
            || info.size.y <= 0.0
        {
            return Err(fail("invalid profile geometry"));
        }
        if info.scripts.len() > usize::from(UNMAPPED) {
            return Err(fail("too many animation rows"));
        }
        if info.conversion.as_ref() != &hackable_animation_conversion(&info.scripts) {
            return Err(fail("invalid animation conversion table"));
        }
        let mut actions = std::collections::HashSet::new();
        let mut previous = None;
        for script in profile.info.scripts.iter() {
            if usize::from(script.action_id) >= NONANIMATION_END {
                return Err(fail("invalid action ID"));
            }
            if previous != Some(script.action_id) && !actions.insert(script.action_id) {
                return Err(fail("direction rows for an action must be contiguous"));
            }
            previous = Some(script.action_id);
            // This is the loose/custom import contract, NOT SpriteScript's
            // general invariant: original paged rows can have extra frame IDs.
            let count = script.frame_ids.len();
            if count == 0
                || count > usize::from(u16::MAX)
                || [
                    script.delays.len(),
                    script.distances.len(),
                    script.offsets.len(),
                    script.sound_ids.len(),
                ]
                .iter()
                .any(|&len| len != count)
            {
                return Err(fail(
                    "empty animation or inconsistent frame metadata lengths",
                ));
            }
            if ![script.average_speed, script.hotspot.x, script.hotspot.y]
                .iter()
                .all(|value| value.is_finite())
                || script
                    .offsets
                    .iter()
                    .any(|offset| !offset.x.is_finite() || !offset.y.is_finite())
            {
                return Err(fail("non-finite animation geometry"));
            }
            for frame_id in &script.frame_ids {
                if *frame_id as usize >= frame_count {
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

    fn manifest(directions: &[Option<u16>]) -> HackableRhsManifest {
        serde_json::from_value(serde_json::json!({
            "pixel_format": "rgba", "profiles": [{
                "name": "test", "width": 1.0, "height": 1.0, "center_x": 0.0, "center_y": 0.0,
                "rows": directions.iter().enumerate().map(|(index, direction)| serde_json::json!({
                    "action_id": 3, "action_done": 0, "average_speed": 0.0,
                    "direction": direction, "hotspot_x": 0.0, "hotspot_y": 0.0, "path": ".",
                    "frames": [{"file": format!("{index}.png"), "delay": index + 1,
                        "distance": 0, "offset_x": 0.0, "offset_y": 0.0, "sound_id": 0}]
                })).collect::<Vec<_>>()
            }]
        }))
        .unwrap()
    }

    fn build_test_cache(manifest: HackableRhsManifest) -> Result<HackableRhsCache, String> {
        build_hackable_cache_with_reader(
            [0; 32],
            manifest,
            |_, _| {
                Ok((
                    assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                        1,
                        1,
                        &[1, 2, 3, 255],
                        false,
                    ),
                    None,
                ))
            },
            |error| error,
        )
    }

    #[test]
    fn authored_directions_are_ordered_and_missing_directions_keep_encounter_order() {
        let cache = build_test_cache(manifest(&[Some(1), Some(0)])).unwrap();
        assert_eq!(cache.profiles[0].info.scripts[0].delays, [2]);
        assert_eq!(cache.profiles[0].info.scripts[1].delays, [1]);
        let cache = build_test_cache(manifest(&[None, None])).unwrap();
        assert_eq!(cache.profiles[0].info.scripts[0].delays, [1]);
        assert_eq!(cache.profiles[0].info.scripts[1].delays, [2]);
        let encoded = bitcode::encode(&cache);
        let decoded: HackableRhsCache = bitcode::decode(&encoded).unwrap();
        validate_cache_frames("round trip", &decoded).unwrap();
        assert_eq!(bitcode::encode(&decoded), encoded);
    }

    #[test]
    fn duplicate_gapped_and_ambiguous_mixed_directions_fail_before_reading_images() {
        for directions in [&[Some(0), Some(0)][..], &[Some(1)], &[Some(1), None]] {
            let result = build_hackable_cache_with_reader(
                [0; 32],
                manifest(directions),
                |_, _| -> Result<_, String> {
                    panic!("invalid direction layout must fail before image reads")
                },
                |error| error,
            );
            assert!(
                result
                    .unwrap_err()
                    .contains("duplicate or missing direction")
            );
        }
    }

    #[test]
    fn interleaved_actions_are_grouped_without_changing_first_action_order() {
        let mut manifest = manifest(&[None, None, None]);
        manifest.profiles[0].rows[1].action_id = 6;
        let cache = build_test_cache(manifest).unwrap();
        assert_eq!(
            cache.profiles[0]
                .info
                .scripts
                .iter()
                .map(|row| row.action_id)
                .collect::<Vec<_>>(),
            [3, 3, 6]
        );
        assert_eq!(cache.profiles[0].info.conversion[6], 2);
    }

    #[test]
    fn partial_direction_sets_and_original_double_sets_do_not_invent_frames() {
        for count in [1, 2, 16, 32] {
            let directions: Vec<_> = (0..count).map(Some).collect();
            let cache = build_test_cache(manifest(&directions)).unwrap();
            assert_eq!(cache.profiles[0].info.scripts.len(), usize::from(count));
            assert_eq!(cache.frames.len(), usize::from(count));
            // Only offsets 0..count are authored. The importer does not claim
            // that a caller requesting another direction has a valid row.
            assert!(
                cache.profiles[0]
                    .info
                    .scripts
                    .get(usize::from(count))
                    .is_none()
            );
        }
    }

    #[test]
    fn malformed_cached_payloads_and_metadata_are_rejected() {
        let valid = bitcode::encode(&build_test_cache(manifest(&[None])).unwrap());
        let corruptions: &[fn(&mut HackableRhsCache)] = &[
            |cache| {
                cache.frames[0]
                    .rgba_data
                    .as_mut()
                    .unwrap()
                    .pop()
                    .map(|_| ())
                    .unwrap()
            },
            |cache| {
                cache.frames[0].packed_data.pop();
            },
            |cache| cache.frames[0].packed_data.push(0),
            |cache| cache.frames[0].width = 0,
            |cache| cache.profiles[0].info.size.x = f32::NAN,
            |cache| {
                std::sync::Arc::make_mut(&mut cache.profiles[0].info.scripts)[0].offsets[0].x =
                    f32::INFINITY
            },
            |cache| {
                std::sync::Arc::make_mut(&mut cache.profiles[0].info.scripts)[0]
                    .delays
                    .clear();
            },
            |cache| {
                std::sync::Arc::make_mut(&mut cache.profiles[0].info.scripts)[0].frame_ids[0] = 99
            },
            |cache| std::sync::Arc::make_mut(&mut cache.profiles[0].info.conversion)[3] = 99,
            |cache| {
                std::sync::Arc::make_mut(&mut cache.profiles[0].info.scripts)[0].action_id =
                    u16::MAX
            },
        ];
        for (index, corrupt) in corruptions.iter().enumerate() {
            let mut cache = bitcode::decode(&valid).unwrap();
            corrupt(&mut cache);
            assert!(
                validate_cache_frames("corrupt", &cache).is_err(),
                "corruption {index}"
            );
        }
    }
    #[test]
    fn png_decoder_rejects_oversized_headers_before_pixel_decoding() {
        for (width, height, expected) in [
            (65_536, 1, "sprite width exceeds u16"),
            (1, 65_536, "sprite height exceeds u16"),
            (65_535, 65_535, "sprite exceeds"),
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
