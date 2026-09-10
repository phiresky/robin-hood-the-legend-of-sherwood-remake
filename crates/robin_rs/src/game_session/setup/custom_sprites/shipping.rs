//! Portable custom sprites: exact RGB565 four-pixel dictionaries and the
//! shipping adaptive VQ codec. PNGs and disposable caches are not required.
use super::*;
use anyhow::{Context, Result, ensure};
use assets_frame_holder::{RuntimeSprite, TRANSPARENT_COLOR_16};
use robin_assets::sprite_codec::{SpriteGrid, decode_grids_shipping, encode_grids_shipping};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

const MAGIC: &[u8] = b"RHMODVQ1";
// Bound dictionaries below the u16 alphabet limit without lossy quantization.
const GROUP_TILES: usize = 60_000;

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct Bundle {
    metadata: HackableRhsCache,
    groups: Vec<Group>,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct Group {
    dictionary: Vec<[u16; 4]>,
    sizes: Vec<(u16, u16)>,
    blob: Vec<u8>,
}

fn frame_tiles(frame: &RuntimeSprite) -> Result<Vec<[u16; 4]>> {
    ensure!(
        frame.rgba_data.is_none(),
        "VQ export requires legacy_color_keys sprites"
    );
    let width = usize::from(frame.width);
    let stride = width.div_ceil(4) * 4;
    robin_assets::packed_sprite::validate_rle(&frame.packed_data, width, frame.height.into())?;
    let mut tiles = Vec::new();
    let mut position = 0;
    for _ in 0..frame.height {
        let row =
            robin_assets::packed_sprite::read_rle_row(&frame.packed_data, &mut position, width)?;
        let mut pixels = vec![TRANSPARENT_COLOR_16; stride];
        let literals = &frame.packed_data[row.literals];
        pixels[row.first..row.first + literals.len()].copy_from_slice(literals);
        tiles.extend(pixels.as_chunks::<4>().0.iter().copied());
    }
    Ok(tiles)
}

fn encode_group(frames: &[RuntimeSprite]) -> Result<Group> {
    let tiles = frames.iter().map(frame_tiles).collect::<Result<Vec<_>>>()?;
    let mut frequencies = HashMap::<[u16; 4], usize>::new();
    for tile in tiles.iter().flatten() {
        *frequencies.entry(*tile).or_default() += 1;
    }
    // Frequency ordering matches shipping dictionary compaction and gives a
    // deterministic alphabet independent of randomized hash-map iteration.
    let mut dictionary: Vec<_> = frequencies.keys().copied().collect();
    dictionary.sort_by_key(|tile| (std::cmp::Reverse(frequencies[tile]), *tile));
    if dictionary.is_empty() {
        dictionary.push([TRANSPARENT_COLOR_16; 4]);
    }
    let alphabet = u16::try_from(dictionary.len()).context("VQ dictionary exceeds u16")?;
    let indices: HashMap<_, _> = dictionary
        .iter()
        .enumerate()
        .map(|(i, tile)| (*tile, i as u16))
        .collect();
    let grids: Vec<Vec<u16>> = tiles
        .iter()
        .map(|frame| frame.iter().map(|tile| indices[tile]).collect())
        .collect();
    let sizes: Vec<_> = frames
        .iter()
        .map(|frame| (frame.width, frame.height))
        .collect();
    let views: Vec<_> = sizes
        .iter()
        .zip(&grids)
        .map(|(&(width, height), indices)| SpriteGrid {
            cols: width.div_ceil(4),
            rows: height,
            indices,
        })
        .collect();
    // TODO: share dictionaries and temporal/camera references across groups
    // for tighter compression; standalone shipping streams remain lossless.
    let blob = encode_grids_shipping(alphabet, &views, None, None, &vec![None; views.len()])?;
    Ok(Group {
        dictionary,
        sizes,
        blob,
    })
}

fn decode_group(group: Group) -> Result<Vec<RuntimeSprite>> {
    let alphabet = u16::try_from(group.dictionary.len()).context("invalid VQ dictionary size")?;
    ensure!(alphabet > 0, "empty VQ dictionary");
    let dims: Vec<_> = group
        .sizes
        .iter()
        .map(|&(width, height)| (width.div_ceil(4), height))
        .collect();
    let tiles: usize = dims
        .iter()
        .map(|&(w, h)| usize::from(w) * usize::from(h))
        .sum();
    ensure!(tiles <= GROUP_TILES, "VQ group exceeds tile budget");
    let grids = decode_grids_shipping(
        alphabet,
        &dims,
        None,
        None,
        &vec![None; dims.len()],
        &group.blob,
    )?;
    let mut frames = Vec::new();
    for ((width, height), grid) in group.sizes.into_iter().zip(grids) {
        let cols = usize::from(width.div_ceil(4));
        let mut packed_data = Vec::new();
        for y in 0..usize::from(height) {
            let mut pixels = Vec::with_capacity(cols * 4);
            for &index in &grid[y * cols..(y + 1) * cols] {
                pixels.extend_from_slice(
                    group
                        .dictionary
                        .get(usize::from(index))
                        .context("invalid VQ tile index")?,
                );
            }
            let row = &pixels[..usize::from(width)];
            if let Some(first) = row.iter().position(|&pixel| pixel != TRANSPARENT_COLOR_16) {
                let last = row
                    .iter()
                    .rposition(|&pixel| pixel != TRANSPARENT_COLOR_16)
                    .expect("nonempty row");
                packed_data.extend([first as u16, last as u16]);
                packed_data.extend_from_slice(&row[first..=last]);
            } else {
                packed_data.extend([u16::MAX, u16::MAX]);
            }
        }
        frames.push(RuntimeSprite {
            width,
            height,
            packed_data,
            rgba_data: None,
        });
    }
    Ok(frames)
}

pub(super) fn read(path: &Path) -> Result<HackableRhsCache> {
    let compressed = std::fs::read(path).with_context(|| path.display().to_string())?;
    let bytes = zstd::stream::decode_all(compressed.as_slice())?;
    ensure!(bytes.starts_with(MAGIC), "unsupported custom VQ format");
    let mut bundle: Bundle = bitcode::decode(&bytes[MAGIC.len()..])?;
    ensure!(
        bundle.metadata.version == HACKABLE_RHS_CACHE_VERSION,
        "unsupported VQ profile metadata version"
    );
    ensure!(
        bundle.metadata.frames.is_empty() && bundle.metadata.sources.is_empty(),
        "VQ metadata contains source frames"
    );
    for group in bundle.groups {
        bundle.metadata.frames.extend(decode_group(group)?);
    }
    validate_cache_frames(&path.display().to_string(), &bundle.metadata)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(bundle.metadata)
}

/// Encode one legacy-color-key `.rhs.d` directory to `sprites.vq.zst` and
/// verify every frame and animation field through the runtime reader.
pub fn encode_custom_sprite_dir(source: &Path, destination: &Path) -> Result<usize> {
    ensure!(
        !destination.exists(),
        "destination exists: {}",
        destination.display()
    );
    let bytes = std::fs::read(source.join("manifest.json"))?;
    let manifest: HackableRhsManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        matches!(
            manifest.pixel_format,
            HackableRhsPixelFormat::LegacyColorKeys
        ),
        "VQ export requires legacy_color_keys"
    );
    let mut cache = build_hackable_cache(source, hackable_manifest_hash(&bytes), manifest)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let frames = std::mem::take(&mut cache.frames);
    cache.sources.clear();
    let profile_bytes = bitcode::encode(&cache.profiles);
    let mut groups = Vec::new();
    let mut start = 0;
    let mut tiles = 0;
    for (index, frame) in frames.iter().enumerate() {
        let count = usize::from(frame.width.div_ceil(4)) * usize::from(frame.height);
        ensure!(
            count <= GROUP_TILES,
            "frame {index} exceeds VQ group tile budget"
        );
        if index > start && tiles + count > GROUP_TILES {
            groups.push(encode_group(&frames[start..index])?);
            start = index;
            tiles = 0;
        }
        tiles += count;
    }
    if start < frames.len() {
        groups.push(encode_group(&frames[start..])?);
    }
    let mut encoded = MAGIC.to_vec();
    encoded.extend(bitcode::encode(&Bundle {
        metadata: cache,
        groups,
    }));
    let compressed = zstd::stream::encode_all(encoded.as_slice(), 19)?;
    std::fs::write(destination, compressed)?;
    let decoded = read(destination)?;
    ensure!(
        bitcode::encode(&decoded.profiles) == profile_bytes,
        "animation metadata changed"
    );
    ensure!(decoded.frames.len() == frames.len(), "frame count changed");
    for (index, (before, after)) in frames.iter().zip(&decoded.frames).enumerate() {
        ensure!(
            before.width == after.width
                && before.height == after.height
                && before.packed_data == after.packed_data,
            "frame {index} changed during VQ round trip"
        );
    }
    Ok(frames.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_bundle_loads_without_source_pngs_or_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.rhs.d");
        std::fs::create_dir(&source).unwrap();
        let manifest = serde_json::json!({
            "pixel_format": "legacy_color_keys", "profiles": [{
                "name": "Test", "width": 5.0, "height": 1.0,
                "center_x": 2.0, "center_y": 1.0,
                "rows": [{"action_id": 3, "action_done": 0, "average_speed": 1.0,
                    "hotspot_x": 0.0, "hotspot_y": 0.0, "path": ".",
                    "frames": [{"file": "frame.png", "delay": 7, "distance": 2,
                        "offset_x": -3.0, "offset_y": 4.0, "sound_id": 5}]}]
            }]
        });
        std::fs::write(source.join("manifest.json"), manifest.to_string()).unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 5, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[0, 248, 0, 0, 0, 255, 255, 0, 0, 0, 248, 0, 255, 255, 255])
                .unwrap();
        }
        std::fs::write(source.join("frame.png"), png).unwrap();
        let output = directory.path().join("sprites.vq.zst");
        assert_eq!(encode_custom_sprite_dir(&source, &output).unwrap(), 1);
        std::fs::remove_dir_all(source).unwrap();
        let loaded = read(&output).unwrap();
        let script = &loaded.profiles[0].info.scripts[0];
        assert_eq!(script.frame_ids, [0]);
        assert_eq!(script.delays, [7]);
        assert_eq!(script.sound_ids, [5]);
        assert_eq!(loaded.frames[0].width, 5);
        assert_eq!(
            loaded.frames[0].packed_data,
            [1, 4, 0x001f, 0xf800, TRANSPARENT_COLOR_16, 0xffff]
        );
    }

    #[test]
    fn shipping_vq_preserves_odd_width_transparency_and_shadow() {
        let frame = RuntimeSprite {
            width: 5,
            height: 2,
            packed_data: vec![
                1,
                4,
                0x001f,
                0x1234,
                TRANSPARENT_COLOR_16,
                0xabcd,
                u16::MAX,
                u16::MAX,
            ],
            rgba_data: None,
        };
        let group = encode_group(std::slice::from_ref(&frame)).unwrap();
        let result = decode_group(group).unwrap();
        assert_eq!(result[0].packed_data, frame.packed_data);
        assert_eq!((result[0].width, result[0].height), (5, 2));
    }

    #[test]
    fn unsupported_format_and_missing_file_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sprites.vq.zst");
        assert!(read(&path).is_err());
        std::fs::write(
            &path,
            zstd::stream::encode_all(&b"bad format"[..], 1).unwrap(),
        )
        .unwrap();
        assert!(read(&path).is_err());
    }
}
