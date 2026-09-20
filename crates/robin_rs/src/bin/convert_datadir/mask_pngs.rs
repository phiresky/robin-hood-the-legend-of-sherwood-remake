//! Derived, lossless scenery-occlusion images alongside hackable level JSON.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use robin_engine::level_data::RawMask;
use robin_engine::mask::decode_mask_bitmap;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct MaskImage {
    index: usize,
    layer: u16,
    layer_index: usize,
    png: Option<String>,
    mask_type: u8,
    box_top_left: (i16, i16),
    box_size: (i16, i16),
    character_polyline: Option<Vec<(i16, i16)>>,
    projectile_polyline: Option<Vec<(i16, i16)>>,
    obstacle_indices: Vec<u16>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    source: String,
    pixel_values: String,
    masks: Vec<MaskImage>,
}

/// Preserve global array identity even for empty masks; patches reference the
/// separate per-layer ordinal. Sidecars are derived, not runtime replacements.
pub(super) fn export(level_json: &Path, masks: &[RawMask]) -> Result<()> {
    let source = format!(
        "../../{}",
        level_json
            .file_name()
            .context("level JSON filename")?
            .to_string_lossy()
    );
    export_with_source(level_json, masks, source)
}

fn export_with_source(level_json: &Path, masks: &[RawMask], source: String) -> Result<()> {
    let dir = level_json.with_extension("d").join("masks");
    fs::create_dir_all(&dir)?;
    let mut layer_counts = BTreeMap::<u16, usize>::new();
    let mut entries = Vec::with_capacity(masks.len());
    for (index, mask) in masks.iter().enumerate() {
        let layer_index = layer_counts.entry(mask.layer).or_default();
        let (width, height) = mask.box_size;
        let png = if width > 0 && height > 0 {
            let bitmap = decode_mask_bitmap(&mask.mask_data, width as u16, height as u16);
            let pixels: Vec<u8> = bitmap.into_iter().map(|value| value * 255).collect();
            let filename = format!("{index:06}.png");
            let file = fs::File::create(dir.join(&filename))?;
            let mut encoder =
                png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().context("mask PNG header")?;
            writer
                .write_image_data(&pixels)
                .context("mask PNG pixels")?;
            writer.finish().context("mask PNG finish")?;
            Some(filename)
        } else {
            // PNG cannot represent zero-sized masks. Keep their identities and
            // metadata rather than inventing pixels or renumbering later masks.
            None
        };
        entries.push(MaskImage {
            index,
            layer: mask.layer,
            layer_index: *layer_index,
            png,
            mask_type: mask.mask_type,
            box_top_left: mask.box_top_left,
            box_size: mask.box_size,
            character_polyline: mask.character_polyline.clone(),
            projectile_polyline: mask.projectile_polyline.clone(),
            obstacle_indices: mask.obstacle_indices.clone(),
        });
        *layer_index += 1;
    }
    super::write_json_pretty(&dir.join("manifest.json"), &Manifest {
        version: 1,
        source,
        pixel_values: "0 = uncovered; 255 = scenery occludes actor; local pixels + box_top_left = map pixels".into(),
        masks: entries,
    })
}

pub(super) fn backfill(input: &Path, output: &Path) -> Result<()> {
    let levels = super::find_data_dir(input)?.join("Levels");
    let levels =
        super::resolve_case_insensitive(&levels).context("finding hackable Levels directory")?;
    let mut files = Vec::new();
    super::collect_files_recursive(&levels, &mut files)?;
    files.sort();
    let mut count = 0;
    for file in files {
        if !file
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".rhp.json")
        {
            continue;
        }
        // Read only the mask field, allowing sidecar upgrades independently of
        // other descriptor schema changes. The source bytes are never rewritten.
        #[derive(Serialize, Deserialize)]
        struct MaskSource {
            masks: Vec<RawMask>,
        }
        let source: MaskSource = serde_json::from_slice(&fs::read(&file)?)
            .with_context(|| format!("reading masks from {}", file.display()))?;
        let destination = output.join("Data/Levels").join(file.strip_prefix(&levels)?);
        let source_path = fs::canonicalize(&file)?;
        if fs::canonicalize(&destination).ok().as_ref() == Some(&source_path) {
            export(&destination, &source.masks)?;
        } else {
            export_with_source(
                &destination,
                &source.masks,
                source_path.to_string_lossy().into_owned(),
            )?;
        }
        tracing::info!(level = %file.display(), masks = source.masks.len(), "exported mask PNG sidecars");
        count += 1;
    }
    if count == 0 {
        bail!("no .rhp.json levels found in {}", levels.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_all_levels_in_place_preserves_source_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let levels = temp.path().join("Data/Levels");
        fs::create_dir_all(&levels).unwrap();
        let source = br#"{ "masks": [], "unrelated": "preserve formatting and fields" }"#;
        for name in ["First.rhp.json", "SECOND.RHP.JSON"] {
            fs::write(levels.join(name), source).unwrap();
        }
        backfill(temp.path(), temp.path()).unwrap();
        for name in ["First.rhp.json", "SECOND.RHP.JSON"] {
            assert_eq!(fs::read(levels.join(name)).unwrap(), source);
            assert!(
                levels
                    .join(name)
                    .with_extension("d")
                    .join("masks/manifest.json")
                    .exists()
            );
        }
        let separate = temp.path().join("sidecars");
        backfill(temp.path(), &separate).unwrap();
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(separate.join("Data/Levels/First.rhp.d/masks/manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(fs::read(manifest.source).unwrap(), source);
    }

    #[test]
    fn png_preserves_bits_crop_and_both_mask_identities() {
        let temp = tempfile::tempdir().unwrap();
        let mask = RawMask {
            layer: 3,
            mask_type: 1,
            character_polyline: None,
            projectile_polyline: None,
            box_top_left: (-2, 71),
            box_size: (9, 2),
            mask_data: vec![3, 2, 0x81, 0x80, 2, 0x82, 0x55],
            obstacle_indices: vec![42],
        };
        let mut other = mask.clone();
        other.layer = 1;
        let mut empty = mask.clone();
        empty.box_size = (0, 0);
        export(&temp.path().join("test.rhp.json"), &[mask, other, empty]).unwrap();
        let dir = temp.path().join("test.rhp.d/masks");
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest
                .masks
                .iter()
                .map(|m| (m.index, m.layer, m.layer_index))
                .collect::<Vec<_>>(),
            [(0, 3, 0), (1, 1, 0), (2, 3, 1)]
        );
        assert_eq!(manifest.masks[0].box_top_left, (-2, 71));
        assert!(manifest.masks[2].png.is_none());
        let mut reader = png::Decoder::new(std::io::BufReader::new(
            fs::File::open(dir.join("000000.png")).unwrap(),
        ))
        .read_info()
        .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!(
            (info.width, info.height, info.color_type),
            (9, 2, png::ColorType::Grayscale)
        );
        assert_eq!(
            &bytes[..info.buffer_size()],
            &[
                255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0
            ]
        );
    }
}
