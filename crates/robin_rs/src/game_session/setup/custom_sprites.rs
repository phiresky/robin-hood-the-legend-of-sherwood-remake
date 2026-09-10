//! Custom sprite decoding and disposable cache policy.
mod shipping;
use super::error::ResourcePreparationError;
use super::localization::read_optional_json;
use robin_assets::frame_holder as assets_frame_holder;
use robin_engine::coordinates::{SpriteAnchor, SpriteFrameOffset, SpriteLocalPoint, SpriteSize};
use robin_engine::sprite_script::{NONANIMATION_END, SpriteInfo, SpriteScript, UNMAPPED};
use robin_engine::{campaign::Campaign, profiles as engine_profiles, sbfile as engine_sbfile};
pub use shipping::encode_custom_sprite_dir;

#[derive(Debug, serde::Deserialize)]
struct HackableRhsManifest {
    pixel_format: HackableRhsPixelFormat,
    profiles: Vec<HackableRhsProfile>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum HackableRhsPixelFormat {
    Rgba,
    LegacyColorKeys,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsProfile {
    name: String,
    width: f32,
    height: f32,
    center_x: f32,
    center_y: f32,
    rows: Vec<HackableRhsRow>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsRow {
    action_id: u16,
    action_done: u16,
    average_speed: f32,
    #[serde(default, rename = "direction")]
    _direction: u16,
    hotspot_x: f32,
    hotspot_y: f32,
    path: String,
    frames: Vec<HackableRhsFrame>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsFrame {
    file: String,
    delay: u16,
    distance: u16,
    offset_x: f32,
    offset_y: f32,
    sound_id: u16,
}

const HACKABLE_RHS_CACHE_VERSION: u32 = 2;
// Retain the original filename so v1 caches can be repaired in place without
// decoding hundreds of thousands of source PNGs again.
const HACKABLE_RHS_CACHE_FILE: &str = ".robin-rhs-cache-v1.zst";

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCache {
    version: u32,
    manifest_hash: [u8; 32],
    sources: Vec<HackableRhsCacheSource>,
    frames: Vec<assets_frame_holder::RuntimeSprite>,
    profiles: Vec<HackableRhsCacheProfile>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCacheSource {
    relative_path: String,
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCacheProfile {
    name: String,
    info: SpriteInfo,
}

fn overlay_roots(files: &engine_sbfile::SbFileSystem) -> Vec<std::path::PathBuf> {
    files
        .overlay_paths()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}

fn decode_png_rgba(
    path: &std::path::Path,
) -> Result<(u16, u16, Vec<u8>), ResourcePreparationError> {
    let bytes = std::fs::read(path)
        .map_err(|error| ResourcePreparationError::unavailable(path.display(), error))?;
    decode_png_rgba_bytes(&bytes, &path.display().to_string())
        .map_err(|error| ResourcePreparationError::malformed(path.display(), error))
}

fn decode_png_rgba_bytes(bytes: &[u8], source: &str) -> Result<(u16, u16, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("decode {source}: {e}"))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("unknown PNG output size for {source}"))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("read frame {source}: {e}"))?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for px in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for &value in data {
                out.extend_from_slice(&[value, value, value, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for px in data.as_chunks::<2>().0 {
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
    let width =
        u16::try_from(info.width).map_err(|_| format!("sprite width exceeds u16 for {source}"))?;
    let height = u16::try_from(info.height)
        .map_err(|_| format!("sprite height exceeds u16 for {source}"))?;
    Ok((width, height, rgba))
}

fn current_hackable_character_filenames(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    files: &engine_sbfile::SbFileSystem,
) -> Result<Option<std::collections::HashSet<String>>, ResourcePreparationError> {
    let mission_index = campaign.current_mission_idx.ok_or_else(|| {
        ResourcePreparationError::MissingAuthority(
            "custom sprites require a current mission".into(),
        )
    })?;
    let mission = campaign
        .missions
        .get(mission_index)
        .ok_or_else(|| {
            ResourcePreparationError::malformed(
                "campaign",
                format!("missing mission {mission_index}"),
            )
        })?
        .profile(profiles);
    let descriptor_path =
        robin_engine::level_data::hackable_level_descriptor_path(&mission.mission_filename);
    let Some(descriptor) = read_optional_json::<robin_engine::level_data::HackableLevelDescriptor>(
        files,
        &descriptor_path,
    )?
    else {
        return Ok(None);
    };
    let mut filenames = std::collections::HashSet::new();
    for soldier in descriptor.soldiers {
        let profile = match soldier.profile {
            robin_engine::level_data::HackableSoldierProfile::Identifier(identifier) => {
                let index = profiles
                    .soldier_idx_by_identifier(&identifier)
                    .map_err(|error| {
                        ResourcePreparationError::malformed(
                            &descriptor_path,
                            format!("soldier {identifier:?}: {error}"),
                        )
                    })?;
                profiles.get_soldier(index)
            }
            robin_engine::level_data::HackableSoldierProfile::LegacyIndex(index) => {
                profiles.get_soldier(index)
            }
        };
        let profile = profile.ok_or_else(|| {
            ResourcePreparationError::malformed(
                &descriptor_path,
                "soldier references a missing profile",
            )
        })?;
        filenames.insert(profile.filename.clone());
    }
    Ok(Some(filenames))
}

fn hackable_manifest_hash(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

fn hackable_source_stamp(
    root: &std::path::Path,
    relative_path: &str,
) -> Result<HackableRhsCacheSource, String> {
    let path = root.join(relative_path);
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

fn hackable_cache_sources_are_current(
    root: &std::path::Path,
    cache: &HackableRhsCache,
    manifest_hash: [u8; 32],
) -> bool {
    cache.manifest_hash == manifest_hash
        && cache.sources.iter().all(|source| {
            hackable_source_stamp(root, &source.relative_path).is_ok_and(|current| {
                current.len == source.len
                    && current.modified_secs == source.modified_secs
                    && current.modified_nanos == source.modified_nanos
            })
        })
}

fn read_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
) -> Option<HackableRhsCache> {
    let cache_path = root.join(HACKABLE_RHS_CACHE_FILE);
    let compressed = match std::fs::read(&cache_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Failed to read {}: {error}", cache_path.display());
            return None;
        }
    };
    let encoded = match zstd::stream::decode_all(std::io::Cursor::new(compressed)) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("Failed to decompress {}: {error}", cache_path.display());
            return None;
        }
    };
    let mut cache: HackableRhsCache = match bitcode::decode(&encoded) {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!("Failed to decode {}: {error}", cache_path.display());
            return None;
        }
    };
    if !hackable_cache_sources_are_current(root, &cache, manifest_hash) {
        return None;
    }
    match cache.version {
        HACKABLE_RHS_CACHE_VERSION => Some(cache),
        1 => {
            // Version 1 eagerly installed walking fallbacks while reading
            // rows. A later explicit RunningUpright row therefore could not
            // replace the fallback and resolved to WalkingUpright. Rebuild
            // only the small action tables; packed pixels remain valid.
            for profile in &mut cache.profiles {
                profile.info.conversion = std::sync::Arc::new(hackable_animation_conversion(
                    profile.info.scripts.as_ref(),
                ));
            }
            cache.version = HACKABLE_RHS_CACHE_VERSION;
            if let Err(error) = write_hackable_cache(root, &cache) {
                tracing::warn!("Failed to upgrade {}: {error}", cache_path.display());
            }
            Some(cache)
        }
        version => {
            tracing::warn!(
                "Ignoring {} with unsupported cache version {version}",
                cache_path.display()
            );
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_hackable_cache(root: &std::path::Path, cache: &HackableRhsCache) -> Result<(), String> {
    use std::io::Write as _;

    let encoded = bitcode::encode(cache);
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(encoded), 3)
        .map_err(|error| format!("compress hackable sprite cache: {error}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(|error| {
        format!(
            "create hackable sprite cache in {}: {error}",
            root.display()
        )
    })?;
    temporary
        .write_all(&compressed)
        .map_err(|error| format!("write hackable sprite cache in {}: {error}", root.display()))?;
    temporary
        .persist(root.join(HACKABLE_RHS_CACHE_FILE))
        .map_err(|error| {
            format!(
                "persist hackable sprite cache in {}: {error}",
                root.display()
            )
        })?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_hackable_cache(_root: &std::path::Path, _cache: &HackableRhsCache) -> Result<(), String> {
    Ok(())
}

fn hackable_animation_conversion(scripts: &[SpriteScript]) -> Vec<u16> {
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

fn build_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
    manifest: HackableRhsManifest,
) -> Result<HackableRhsCache, ResourcePreparationError> {
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
                    let frame_path = root.join(&relative_path);
                    let (width, height, rgba) = decode_png_rgba(&frame_path)?;
                    let source = hackable_source_stamp(root, &relative_path).map_err(|error| {
                        ResourcePreparationError::unavailable(frame_path.display(), error)
                    })?;
                    let local_id = frames.len() as u32;
                    frames.push(assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                        width,
                        height,
                        &rgba,
                        legacy_color_keys,
                    ));
                    sources.push(source);
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

/// Fully decoded custom content. Preparation never mutates live sprite owners.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedCustomSprites {
    batches: Vec<(String, HackableRhsCache)>,
}

impl PreparedCustomSprites {
    pub(super) fn install(
        self,
        frames: &mut assets_frame_holder::FrameHolder,
        scriptor: &mut robin_engine::sprite_script::SpriteScriptor,
    ) -> Result<(), ResourcePreparationError> {
        // Validate the entire handoff before appending any frame or profile.
        for (filename, cache) in &self.batches {
            validate_cache_frames(filename, cache)?;
        }
        for (filename, cache) in self.batches {
            let frame_ids: Vec<_> = cache
                .frames
                .into_iter()
                .map(|frame| frames.append_runtime_sprite(frame))
                .collect();
            for profile in cache.profiles {
                let mut info = profile.info;
                for script in std::sync::Arc::make_mut(&mut info.scripts) {
                    for frame_id in &mut script.frame_ids {
                        *frame_id = frame_ids[*frame_id as usize];
                    }
                }
                let key = format!("{filename}/{}", profile.name);
                scriptor.insert(key.clone(), info);
                tracing::info!("Loaded hackable character profile {key}");
            }
        }
        Ok(())
    }
}

fn validate_cache_frames(
    filename: &str,
    cache: &HackableRhsCache,
) -> Result<(), ResourcePreparationError> {
    for profile in &cache.profiles {
        for script in profile.info.scripts.iter() {
            for frame_id in &script.frame_ids {
                if *frame_id as usize >= cache.frames.len() {
                    return Err(ResourcePreparationError::malformed(
                        filename,
                        format!(
                            "sprite profile {} references missing local frame {frame_id}",
                            profile.name
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn prepare_custom_character_dirs(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    files: &engine_sbfile::SbFileSystem,
) -> Result<PreparedCustomSprites, ResourcePreparationError> {
    let mission_filenames = current_hackable_character_filenames(campaign, profiles, files)?;
    let mut batches = Vec::new();
    for root in overlay_roots(files) {
        let chars = root.join("Data/Characters");
        let mission_scoped = chars.join("mission-scoped.json").is_file();
        let entries = match std::fs::read_dir(&chars) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ResourcePreparationError::unavailable(
                    chars.display(),
                    error,
                ));
            }
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| ResourcePreparationError::unavailable(chars.display(), error))?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(filename) = name.strip_suffix(".rhs.d") else {
                continue;
            };
            if mission_scoped
                && !mission_filenames
                    .as_ref()
                    .is_some_and(|filenames| filenames.contains(filename))
            {
                continue;
            }
            let shipping_path = path.join("sprites.vq.zst");
            if shipping_path.is_file() {
                let cache = shipping::read(&shipping_path).map_err(|error| {
                    ResourcePreparationError::malformed(shipping_path.display(), error)
                })?;
                validate_cache_frames(filename, &cache)?;
                batches.push((filename.to_owned(), cache));
                continue;
            }
            let manifest_path = path.join("manifest.json");
            // A discovered authored sprite directory must contain its manifest.
            let manifest_bytes = std::fs::read(&manifest_path).map_err(|error| {
                ResourcePreparationError::unavailable(manifest_path.display(), error)
            })?;
            let manifest_hash = hackable_manifest_hash(&manifest_bytes);
            if let Some(cache) = read_hackable_cache(&path, manifest_hash) {
                match validate_cache_frames(filename, &cache) {
                    Ok(()) => {
                        batches.push((filename.to_owned(), cache));
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Ignoring invalid disposable sprite cache; rebuilding from source")
                    }
                }
            }
            let manifest: HackableRhsManifest =
                serde_json::from_slice(&manifest_bytes).map_err(|error| {
                    ResourcePreparationError::malformed(manifest_path.display(), error)
                })?;
            let cache = build_hackable_cache(&path, manifest_hash, manifest)?;
            // Persistence is an optimization; decoded authored content remains valid.
            if let Err(error) = write_hackable_cache(&path, &cache) {
                tracing::warn!("Failed to cache hackable sprites for {filename}: {error}");
            }
            batches.push((filename.to_owned(), cache));
        }
    }
    Ok(PreparedCustomSprites { batches })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_sprite_io_and_decode_failures_are_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.png");
        assert!(matches!(
            decode_png_rgba(&path),
            Err(ResourcePreparationError::Unavailable { .. })
        ));
        std::fs::write(&path, b"not a png").unwrap();
        assert!(matches!(
            decode_png_rgba(&path),
            Err(ResourcePreparationError::Malformed { .. })
        ));
    }

    #[test]
    fn invalid_cache_install_does_not_append_partial_frames() {
        let cache = HackableRhsCache {
            version: HACKABLE_RHS_CACHE_VERSION,
            manifest_hash: [0; 32],
            sources: Vec::new(),
            frames: vec![assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                1,
                1,
                &[0, 0, 0, 255],
                false,
            )],
            profiles: vec![HackableRhsCacheProfile {
                name: "invalid".into(),
                info: SpriteInfo {
                    scripts: std::sync::Arc::new(vec![SpriteScript {
                        frame_ids: vec![99],
                        ..SpriteScript::default()
                    }]),
                    conversion: std::sync::Arc::new(vec![]),
                    size: SpriteSize::new(1.0, 1.0),
                    center: SpriteAnchor::new(0.0, 0.0),
                },
            }],
        };
        let prepared = PreparedCustomSprites {
            batches: vec![("invalid.rhs.d".into(), cache)],
        };
        let mut holder = assets_frame_holder::FrameHolder::new();
        let mut scriptor = robin_engine::sprite_script::SpriteScriptor::new();
        assert!(matches!(
            prepared.install(&mut holder, &mut scriptor),
            Err(ResourcePreparationError::Malformed { .. })
        ));
        assert_eq!(holder.num_sprites(), 0);
    }

    #[test]
    fn missing_authored_frame_rejects_whole_prepared_cache() {
        let dir = tempfile::tempdir().unwrap();
        let manifest: HackableRhsManifest = serde_json::from_value(serde_json::json!({
            "pixel_format": "rgba", "profiles": [{
                "name": "test", "width": 1.0, "height": 1.0, "center_x": 0.0, "center_y": 0.0,
                "rows": [{ "action_id": 3, "action_done": 0, "average_speed": 0.0,
                    "hotspot_x": 0.0, "hotspot_y": 0.0, "path": ".",
                    "frames": [{ "file": "missing.png", "delay": 1, "distance": 0,
                        "offset_x": 0.0, "offset_y": 0.0, "sound_id": 0 }]}]
            }]
        }))
        .unwrap();
        assert!(matches!(
            build_hackable_cache(dir.path(), [0; 32], manifest),
            Err(ResourcePreparationError::Unavailable { .. })
        ));
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

    #[test]
    fn hackable_sprite_cache_round_trips_and_invalidates_changed_sources() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("frame.png"), b"source").unwrap();
        let manifest_hash = hackable_manifest_hash(b"manifest");
        let cache = HackableRhsCache {
            version: HACKABLE_RHS_CACHE_VERSION,
            manifest_hash,
            sources: vec![hackable_source_stamp(directory.path(), "frame.png").unwrap()],
            frames: vec![assets_frame_holder::RuntimeSprite {
                width: 2,
                height: 1,
                packed_data: vec![0, 1, 0x1234, 0x5678],
                rgba_data: None,
            }],
            profiles: Vec::new(),
        };

        write_hackable_cache(directory.path(), &cache).unwrap();
        let decoded = read_hackable_cache(directory.path(), manifest_hash).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.frames[0].packed_data, cache.frames[0].packed_data);

        std::fs::write(directory.path().join("frame.png"), b"source changed").unwrap();
        assert!(read_hackable_cache(directory.path(), manifest_hash).is_none());
        assert!(read_hackable_cache(directory.path(), [7; 32]).is_none());
    }

    #[test]
    fn version_one_hackable_cache_repairs_walking_over_run_alias() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("frame.png"), b"source").unwrap();
        let manifest_hash = hackable_manifest_hash(b"manifest");
        let scripts = vec![
            SpriteScript {
                action_id: 6,
                ..SpriteScript::default()
            },
            SpriteScript {
                action_id: 10,
                ..SpriteScript::default()
            },
        ];
        let mut broken_conversion = vec![UNMAPPED; NONANIMATION_END];
        broken_conversion[6] = 0;
        broken_conversion[10] = 0;
        let cache = HackableRhsCache {
            version: 1,
            manifest_hash,
            sources: vec![hackable_source_stamp(directory.path(), "frame.png").unwrap()],
            frames: Vec::new(),
            profiles: vec![HackableRhsCacheProfile {
                name: "test".to_owned(),
                info: SpriteInfo {
                    scripts: std::sync::Arc::new(scripts),
                    conversion: std::sync::Arc::new(broken_conversion),
                    size: SpriteSize::new(1.0, 1.0),
                    center: SpriteAnchor::new(0.0, 0.0),
                },
            }],
        };
        write_hackable_cache(directory.path(), &cache).unwrap();

        let upgraded = read_hackable_cache(directory.path(), manifest_hash).unwrap();
        assert_eq!(upgraded.version, HACKABLE_RHS_CACHE_VERSION);
        assert_eq!(upgraded.profiles[0].info.conversion[6], 0);
        assert_eq!(upgraded.profiles[0].info.conversion[10], 1);
    }
}
