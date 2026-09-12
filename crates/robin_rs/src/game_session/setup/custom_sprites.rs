//! Custom sprite decoding and disposable cache policy.
use super::error::ResourcePreparationError;
use super::localization::read_optional_json;
#[cfg(test)]
use robin_assets::custom_sprites::HackableRhsCacheProfile;
use robin_assets::custom_sprites::{
    HACKABLE_RHS_CACHE_VERSION, HackableRhsCache, HackableRhsManifest,
    build_hackable_cache_with_reader, decode_png_rgba_bytes, family, hackable_manifest_hash,
    hackable_source_stamp, shipping,
};
use robin_assets::frame_holder as assets_frame_holder;
#[cfg(test)]
use robin_engine::coordinates::{SpriteAnchor, SpriteSize};
#[cfg(test)]
use robin_engine::sprite_script::{NONANIMATION_END, SpriteInfo, SpriteScript, UNMAPPED};
use robin_engine::{campaign::Campaign, profiles as engine_profiles, sbfile as engine_sbfile};

// Retain the original filename so v1 caches can be repaired in place.
const HACKABLE_RHS_CACHE_FILE: &str = ".robin-rhs-cache-v1.zst";
// Per-family cache budgets, not limits on authored sprite content. Oversized
// families still load from source; their disposable cache is simply not used.
const HACKABLE_RHS_CACHE_COMPRESSED_LIMIT: u64 = 256 * 1024 * 1024;
const HACKABLE_RHS_CACHE_DECODED_LIMIT: u64 = 512 * 1024 * 1024;
// Our level-3 encoder needs far less than this. Bound decoder working memory
// independently of the amount of output a compressed stream produces.
const HACKABLE_RHS_CACHE_WINDOW_LOG_MAX: u32 = 27;

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

fn hackable_cache_sources_are_current(
    root: &std::path::Path,
    cache: &HackableRhsCache,
    manifest_hash: [u8; 32],
) -> bool {
    // TODO: Support explicit content verification for tools that preserve source
    // length and mtime. Avoid rehashing every PNG on ordinary cached startup.
    cache.manifest_hash == manifest_hash
        && cache.sources.iter().all(|source| {
            hackable_source_stamp(root, &source.relative_path).is_ok_and(|current| {
                current.len == source.len
                    && current.modified_secs == source.modified_secs
                    && current.modified_nanos == source.modified_nanos
            })
        })
}

fn read_cache_bytes_limited(reader: impl std::io::Read, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("sprite cache exceeds {limit}-byte limit"),
        ));
    }
    Ok(bytes)
}

fn decompress_cache_limited(compressed: &[u8], limit: u64) -> std::io::Result<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(compressed)?;
    decoder.window_log_max(HACKABLE_RHS_CACHE_WINDOW_LOG_MAX)?;
    read_cache_bytes_limited(decoder, limit)
}

fn read_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
) -> Option<HackableRhsCache> {
    let cache_path = root.join(HACKABLE_RHS_CACHE_FILE);
    let compressed = match std::fs::File::open(&cache_path).and_then(|file| {
        if file.metadata()?.len() > HACKABLE_RHS_CACHE_COMPRESSED_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sprite cache exceeds compressed size limit",
            ));
        }
        // Enforce the limit while reading as well: the file can grow after stat.
        read_cache_bytes_limited(file, HACKABLE_RHS_CACHE_COMPRESSED_LIMIT)
    }) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Failed to read {}: {error}", cache_path.display());
            return None;
        }
    };
    let encoded = match decompress_cache_limited(&compressed, HACKABLE_RHS_CACHE_DECODED_LIMIT) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("Failed to decompress {}: {error}", cache_path.display());
            return None;
        }
    };
    let cache: HackableRhsCache = match bitcode::decode(&encoded) {
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
        // Earlier caches discarded authored directions. Rebuild from the
        // manifest: the cached row order cannot recover that information.
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
    if encoded.len() as u64 > HACKABLE_RHS_CACHE_DECODED_LIMIT {
        return Err(
            "sprite cache exceeds decoded size limit; retaining source-only loading".into(),
        );
    }
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(encoded), 3)
        .map_err(|error| format!("compress hackable sprite cache: {error}"))?;
    if compressed.len() as u64 > HACKABLE_RHS_CACHE_COMPRESSED_LIMIT {
        return Err(
            "sprite cache exceeds compressed size limit; retaining source-only loading".into(),
        );
    }
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
    robin_assets::custom_sprites::validate_cache_frames(filename, cache)
        .map_err(|error| ResourcePreparationError::malformed(filename, error))
}

pub(super) fn prepare_custom_character_dirs(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    files: &engine_sbfile::SbFileSystem,
) -> Result<PreparedCustomSprites, ResourcePreparationError> {
    let mission_filenames = current_hackable_character_filenames(campaign, profiles, files)?;
    prepare_overlay_characters(files, mission_filenames.as_ref())
}

fn overlay_bytes(
    files: &engine_sbfile::SbFileSystem,
    source: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, ResourcePreparationError> {
    files
        .read_overlay(source, path)
        .map_err(|error| ResourcePreparationError::unavailable(format!("{source}/{path}"), error))
}

fn required_overlay_bytes(
    files: &engine_sbfile::SbFileSystem,
    source: &str,
    path: &str,
) -> Result<Vec<u8>, ResourcePreparationError> {
    overlay_bytes(files, source, path)?.ok_or_else(|| {
        ResourcePreparationError::unavailable(format!("{source}/{path}"), "required file missing")
    })
}

fn prepare_overlay_characters(
    files: &engine_sbfile::SbFileSystem,
    mission_filenames: Option<&std::collections::HashSet<String>>,
) -> Result<PreparedCustomSprites, ResourcePreparationError> {
    let mut batches = Vec::new();
    for source in files.overlay_sources() {
        let chars = "Data/Characters";
        let mission_scoped =
            overlay_bytes(files, &source, &format!("{chars}/mission-scoped.json"))?.is_some();
        let entries = files.list_overlay_dir(&source, chars).map_err(|error| {
            ResourcePreparationError::unavailable(format!("{source}/{chars}"), error)
        })?;
        for entry in entries {
            let name = entry.name;
            let path = format!("{chars}/{name}");
            if !entry.is_dir && name.ends_with(".sprites.vq.zst") {
                let empty = std::collections::HashSet::new();
                let selected = mission_scoped.then(|| mission_filenames.unwrap_or(&empty));
                let bytes = required_overlay_bytes(files, &source, &path)?;
                batches.extend(
                    family::read_selected_bytes(&bytes, selected)
                        .map_err(|error| ResourcePreparationError::malformed(&path, error))?,
                );
                continue;
            }
            let Some(filename) = name.strip_suffix(".rhs.d").filter(|_| entry.is_dir) else {
                continue;
            };
            if mission_scoped && !mission_filenames.is_some_and(|names| names.contains(filename)) {
                continue;
            }
            let shipping_path = format!("{path}/sprites.vq.zst");
            if let Some(bytes) = overlay_bytes(files, &source, &shipping_path)? {
                let cache = shipping::read_bytes(&bytes, &shipping_path)
                    .map_err(|error| ResourcePreparationError::malformed(&shipping_path, error))?;
                validate_cache_frames(filename, &cache)?;
                batches.push((filename.to_owned(), cache));
                continue;
            }
            let manifest_path = format!("{path}/manifest.json");
            let manifest_bytes = required_overlay_bytes(files, &source, &manifest_path)?;
            let manifest_hash = hackable_manifest_hash(&manifest_bytes);
            let cache_root = files
                .overlay_directory(&source)
                .and_then(|root| engine_sbfile::resolve_case_insensitive(&root.join(&path)));
            if let Some(cache) = cache_root
                .as_ref()
                .and_then(|root| read_hackable_cache(root, manifest_hash))
            {
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
            let manifest: HackableRhsManifest = serde_json::from_slice(&manifest_bytes)
                .map_err(|error| ResourcePreparationError::malformed(&manifest_path, error))?;
            let cache = build_hackable_cache_with_reader(
                manifest_hash,
                manifest,
                |relative, legacy| {
                    let frame_path = format!("{path}/{relative}");
                    let bytes = required_overlay_bytes(files, &source, &frame_path)?;
                    let (width, height, rgba) = decode_png_rgba_bytes(&bytes, &frame_path)
                        .map_err(|error| ResourcePreparationError::malformed(&frame_path, error))?;
                    let stamp = cache_root
                        .as_ref()
                        .map(|root| hackable_source_stamp(root, relative))
                        .transpose()
                        .map_err(|error| {
                            ResourcePreparationError::unavailable(&frame_path, error)
                        })?;
                    Ok((
                        assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                            width, height, &rgba, legacy,
                        ),
                        stamp,
                    ))
                },
                |error| ResourcePreparationError::malformed(&manifest_path, error),
            )?;
            if let Some(root) = cache_root
                && let Err(error) = write_hackable_cache(&root, &cache)
            {
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

    fn isolated_files() -> engine_sbfile::SbFileSystem {
        engine_sbfile::SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()))
    }

    fn archive_directory(root: &std::path::Path, prefix: &str) -> Vec<u8> {
        use std::io::Write;
        fn append(
            root: &std::path::Path,
            path: &std::path::Path,
            prefix: &str,
            writer: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
        ) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    append(root, &path, prefix, writer);
                } else {
                    writer
                        .start_file(
                            format!("{prefix}{}", path.strip_prefix(root).unwrap().display()),
                            zip::write::SimpleFileOptions::default(),
                        )
                        .unwrap();
                    writer.write_all(&std::fs::read(path).unwrap()).unwrap();
                }
            }
        }
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        append(root, root, prefix, &mut writer);
        writer.finish().unwrap().into_inner()
    }

    fn decoded_digest(mut prepared: PreparedCustomSprites) -> [u8; 32] {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        prepared.batches.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, mut cache) in prepared.batches {
            // Timestamps and source paths belong only to disposable disk caches.
            cache.sources.clear();
            hash.update(&bitcode::encode(&(name, cache)));
        }
        hash.finalize().into()
    }

    #[test]
    fn png_and_both_vq_formats_load_identically_from_directories_and_archives() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("png");
        let character = source.join("Data/Characters/Knight.rhs.d");
        std::fs::create_dir_all(&character).unwrap();
        let manifest = serde_json::json!({"pixel_format":"legacy_color_keys", "profiles":[{
            "name":"test", "width":4.0,"height":1.0,"center_x":0.0,"center_y":0.0,
            "rows":[{"action_id":3,"action_done":0,"average_speed":0.0,"hotspot_x":0.0,"hotspot_y":0.0,"path":".",
            "frames":[{"file":"Frame.PNG","delay":1,"distance":0,"offset_x":0.0,"offset_y":0.0,"sound_id":0}]}]}]});
        std::fs::write(
            character.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 5, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 248, 0, 0, 0, 255, 255, 0, 0, 0, 248, 0, 255, 255, 255])
                .unwrap();
        }
        std::fs::write(character.join("Frame.PNG"), png).unwrap();
        let v1 = temp.path().join("v1");
        let v1_character = v1.join("Data/Characters/Knight.rhs.d");
        std::fs::create_dir_all(&v1_character).unwrap();
        shipping::encode_custom_sprite_dir(&character, &v1_character.join("sprites.vq.zst"))
            .unwrap();
        let family = temp.path().join("family");
        std::fs::create_dir_all(family.join("Data/Characters")).unwrap();
        family::encode_custom_sprite_family(
            &[character.clone()],
            &family.join("Data/Characters/knights.sprites.vq.zst"),
        )
        .unwrap();
        let mut expected = None;
        for root in [&source, &v1, &family] {
            let files = isolated_files();
            assert_eq!(files.add_overlay_path(root.to_str().unwrap()), Ok(()));
            let digest = decoded_digest(prepare_overlay_characters(&files, None).unwrap());
            if let Some(expected) = expected {
                assert_eq!(digest, expected);
            }
            expected = Some(digest);
            for prefix in ["", "Wrapped/"] {
                let archive = isolated_files();
                assert_eq!(
                    archive.add_overlay_zip_bytes_for_mission(
                        "test",
                        archive_directory(root, prefix).into(),
                        None
                    ),
                    Ok(())
                );
                assert_eq!(
                    decoded_digest(prepare_overlay_characters(&archive, None).unwrap()),
                    digest
                );
            }
        }
        std::fs::write(source.join("Data/Characters/mission-scoped.json"), b"{}").unwrap();
        let files = isolated_files();
        assert_eq!(
            files.add_overlay_zip_bytes_for_mission(
                "scoped",
                archive_directory(&source, "").into(),
                None
            ),
            Ok(())
        );
        assert!(
            prepare_overlay_characters(&files, None)
                .unwrap()
                .batches
                .is_empty()
        );
        let selected = std::collections::HashSet::from(["Knight".to_owned()]);
        assert_eq!(
            prepare_overlay_characters(&files, Some(&selected))
                .unwrap()
                .batches
                .len(),
            1
        );
        std::fs::remove_file(character.join("Frame.PNG")).unwrap();
        let files = isolated_files();
        assert_eq!(
            files.add_overlay_zip_bytes_for_mission(
                "missing",
                archive_directory(&source, "").into(),
                None
            ),
            Ok(())
        );
        assert!(prepare_overlay_characters(&files, Some(&selected)).is_err());
    }

    #[test]
    #[ignore = "requires FABRI18_MOD_DIR and FABRI18_MOD_ZIP fixtures"]
    fn fabri18_archive_matches_directory() {
        let directory = std::env::var("FABRI18_MOD_DIR").expect("FABRI18_MOD_DIR");
        let zip = std::env::var("FABRI18_MOD_ZIP").expect("FABRI18_MOD_ZIP");
        let disk = isolated_files();
        let archive = isolated_files();
        assert_eq!(disk.add_overlay_path(&directory), Ok(()));
        assert_eq!(archive.add_overlay_zip(&zip), Ok(()));
        // Each family is loaded independently to bound fixture memory.
        let mut family_count = 0;
        let mut frame_count = 0;
        for source in disk.overlay_sources() {
            for entry in disk.list_overlay_dir(&source, "Data/Characters").unwrap() {
                if !entry.name.ends_with(".sprites.vq.zst") {
                    continue;
                }
                let path = format!("Data/Characters/{}", entry.name);
                let disk_bytes = disk.read_all(&path).unwrap();
                let zip_bytes = archive.read_all(&path).unwrap();
                assert_eq!(disk_bytes, zip_bytes);
                let batches = family::read_selected_bytes(&disk_bytes, None).unwrap();
                family_count += 1;
                frame_count += batches
                    .iter()
                    .map(|(_, cache)| cache.frames.len())
                    .sum::<usize>();
                let expected = decoded_digest(PreparedCustomSprites { batches });
                let actual = decoded_digest(PreparedCustomSprites {
                    batches: family::read_selected_bytes(&zip_bytes, None).unwrap(),
                });
                assert_eq!(expected, actual, "{}", entry.name);
            }
        }
        assert_eq!(family_count, 9);
        assert_eq!(frame_count, 266_984);
        for path in ["details.json", "Data/Levels/Day/OpenBattlefield.map"] {
            assert_eq!(
                disk.read_all(path).unwrap(),
                archive.read_all(path).unwrap()
            );
        }
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
            batches: vec![
                (
                    "valid.rhs.d".into(),
                    HackableRhsCache {
                        version: HACKABLE_RHS_CACHE_VERSION,
                        manifest_hash: [0; 32],
                        sources: vec![],
                        profiles: vec![],
                        frames: vec![assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                            1,
                            1,
                            &[0, 0, 0, 255],
                            false,
                        )],
                    },
                ),
                ("invalid.rhs.d".into(), cache),
            ],
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
        let manifest = serde_json::json!({
            "pixel_format": "rgba", "profiles": [{
                "name": "test", "width": 1.0, "height": 1.0, "center_x": 0.0, "center_y": 0.0,
                "rows": [{ "action_id": 3, "action_done": 0, "average_speed": 0.0,
                    "hotspot_x": 0.0, "hotspot_y": 0.0, "path": ".",
                    "frames": [{ "file": "missing.png", "delay": 1, "distance": 0,
                        "offset_x": 0.0, "offset_y": 0.0, "sound_id": 0 }]}]
            }]
        });
        let character = dir.path().join("Data/Characters/Test.rhs.d");
        std::fs::create_dir_all(&character).unwrap();
        std::fs::write(
            character.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let files = isolated_files();
        assert_eq!(files.add_overlay_path(dir.path().to_str().unwrap()), Ok(()));
        assert!(matches!(
            prepare_overlay_characters(&files, None),
            Err(ResourcePreparationError::Unavailable { .. })
        ));
        std::fs::write(character.join("missing.png"), b"not a png").unwrap();
        assert!(matches!(
            prepare_overlay_characters(&files, None),
            Err(ResourcePreparationError::Malformed { .. })
        ));
    }
    #[test]
    fn cache_byte_limits_accept_exact_boundary_and_reject_extra_bytes() {
        assert_eq!(read_cache_bytes_limited(&b"abcd"[..], 4).unwrap(), b"abcd");
        assert_eq!(
            read_cache_bytes_limited(&b"abcde"[..], 4)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        let compressed = zstd::stream::encode_all(&b"abcd"[..], 3).unwrap();
        assert_eq!(decompress_cache_limited(&compressed, 4).unwrap(), b"abcd");
        assert_eq!(
            decompress_cache_limited(&compressed, 3).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn cache_decompression_bounds_concatenated_frames_and_rejects_truncation() {
        let compressed = zstd::stream::encode_all(&b"abcd"[..], 3).unwrap();
        assert!(decompress_cache_limited(&compressed[..compressed.len() - 1], 8).is_err());
        let concatenated = [compressed.as_slice(), compressed.as_slice()].concat();
        assert_eq!(
            decompress_cache_limited(&concatenated, 8).unwrap(),
            b"abcdabcd"
        );
        assert_eq!(
            decompress_cache_limited(&concatenated, 7)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn oversized_disposable_cache_rebuilds_from_authored_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let character = directory.path().join("Data/Characters/Knight.rhs.d");
        std::fs::create_dir_all(&character).unwrap();
        let manifest = br#"{"pixel_format":"rgba","profiles":[]}"#;
        std::fs::write(character.join("manifest.json"), manifest).unwrap();
        // Sparse file exercises the production compressed limit without allocating it.
        std::fs::File::create(character.join(HACKABLE_RHS_CACHE_FILE))
            .unwrap()
            .set_len(HACKABLE_RHS_CACHE_COMPRESSED_LIMIT + 1)
            .unwrap();
        assert!(read_hackable_cache(&character, hackable_manifest_hash(manifest)).is_none());
        let files = isolated_files();
        assert_eq!(
            files.add_overlay_path(directory.path().to_str().unwrap()),
            Ok(())
        );
        let prepared = prepare_overlay_characters(&files, None).unwrap();
        assert_eq!(prepared.batches.len(), 1);
        assert!(prepared.batches[0].1.frames.is_empty());
        assert!(read_hackable_cache(&character, hackable_manifest_hash(manifest)).is_some());
    }

    #[test]
    fn malformed_disposable_payload_rebuilds_and_malformed_handoff_is_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let character = directory.path().join("Data/Characters/Knight.rhs.d");
        std::fs::create_dir_all(&character).unwrap();
        let manifest = br#"{"pixel_format":"rgba","profiles":[]}"#;
        std::fs::write(character.join("manifest.json"), manifest).unwrap();
        let make_cache = |corrupt| {
            let mut frame = assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                1,
                1,
                &[0, 0, 0, 255],
                false,
            );
            if corrupt {
                frame.rgba_data.as_mut().unwrap().pop();
            }
            HackableRhsCache {
                version: HACKABLE_RHS_CACHE_VERSION,
                manifest_hash: hackable_manifest_hash(manifest),
                sources: vec![],
                profiles: vec![],
                frames: vec![frame],
            }
        };
        write_hackable_cache(&character, &make_cache(true)).unwrap();
        let files = isolated_files();
        assert_eq!(
            files.add_overlay_path(directory.path().to_str().unwrap()),
            Ok(())
        );
        let prepared = prepare_overlay_characters(&files, None).unwrap();
        assert!(
            prepared.batches[0].1.frames.is_empty(),
            "invalid cached frame must be rebuilt from empty source manifest"
        );
        assert!(
            read_hackable_cache(&character, hackable_manifest_hash(manifest))
                .unwrap()
                .frames
                .is_empty()
        );

        let prepared = PreparedCustomSprites {
            batches: vec![
                ("valid".into(), make_cache(false)),
                ("invalid".into(), make_cache(true)),
            ],
        };
        let mut frames = assets_frame_holder::FrameHolder::new();
        let mut scriptor = robin_engine::sprite_script::SpriteScriptor::new();
        assert!(prepared.install(&mut frames, &mut scriptor).is_err());
        assert_eq!(frames.num_sprites(), 0);
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
    fn earlier_hackable_cache_versions_require_rebuilding_authored_directions() {
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

        assert!(read_hackable_cache(directory.path(), manifest_hash).is_none());
        let mut cache = cache;
        cache.version = 2;
        write_hackable_cache(directory.path(), &cache).unwrap();
        assert!(read_hackable_cache(directory.path(), manifest_hash).is_none());
    }
}
