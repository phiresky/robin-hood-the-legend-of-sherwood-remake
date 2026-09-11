//! Simulation-facing interface metadata shared by graphical and confined loaders.
//! No GPU upload or filesystem authority is introduced here.

use crate::resource_manager::{PictureOpacityMetadata, ResourceManager};
use anyhow::{Context, Result};
use robin_engine::engine::GroundMarkSpriteData;
use robin_engine::resource_ids::*;
use robin_engine::titbit::SpriteRow;

/// Several titbit rows intentionally reuse one original resource.
pub fn titbit_sprite_row_resources() -> &'static [(SpriteRow, i32)] {
    &[
        (SpriteRow::Impact, RHID_ONE_STAR),
        (SpriteRow::OneStar, RHID_ONE_STAR),
        (SpriteRow::TwoStars, RHID_TWO_STARS),
        (SpriteRow::ThreeStars, RHID_THREE_STARS),
        (SpriteRow::FourStars, RHID_FOUR_STARS),
        (SpriteRow::FiveStars, RHID_FIVE_STARS),
        (SpriteRow::QuickActionTitbits, RHID_QUICKACTION_TITBITS),
        (SpriteRow::Smoke, RHID_ONE_STAR),
        (SpriteRow::Water, RHID_TITBIT_WATER),
        (SpriteRow::Lock, RHID_TITBIT_WATER),
        (SpriteRow::EmoticonGrowingQMark, RHID_EMOTICONS_WHAT1),
        (SpriteRow::EmoticonQMark, RHID_EMOTICONS_WHAT2),
        (SpriteRow::EmoticonXMark, RHID_EMOTICONS_ACH),
        (SpriteRow::EmoticonZzz, RHIDEMOTICONS_ZZZ),
        (SpriteRow::EmoticonThunderstorm, RHID_EMOTICONS_ANGRY),
        (SpriteRow::EmoticonCloud, RHID_EMOTICONS_DISAPPOINTED),
        (SpriteRow::EmoticonDrunken, RHID_EMOTICONS_DRUNKEN),
        (SpriteRow::EmoticonSun, RHID_EMOTICONS_HAPPY),
        (SpriteRow::EmoticonKo, RHID_EMOTICONS_KO),
        (SpriteRow::Plouf, RHID_TITBIT_PLOUF),
        (SpriteRow::Ghost, RHID_GHOST_LITTLE_JOHN_SHORT_LEGS),
        (SpriteRow::AppleSmell, RHID_TITBIT_APPLE_SMELL),
        (SpriteRow::Speak, RHID_TITBIT_SPEAK),
        (SpriteRow::DangerPoint, RHID_TITBIT_DANGER_POINT),
        (SpriteRow::Hidden, RHID_TITBIT_HIDDEN),
        (SpriteRow::WorkIconArrows, RHWORKICON_ARROWS),
        (SpriteRow::WorkIconPurses, RHWORKICON_PURSES),
        (SpriteRow::WorkIconStones, RHWORKICON_STONES),
        (SpriteRow::WorkIconApples, RHWORKICON_APPLES),
        (SpriteRow::WorkIconBeer, RHWORKICON_BEER),
        (SpriteRow::WorkIconLegs, RHWORKICON_LEGS),
        (SpriteRow::WorkIconPlants, RHWORKICON_PLANTS),
        (SpriteRow::WorkIconNets, RHWORKICON_NETS),
        (SpriteRow::WorkIconWasps, RHWORKICON_WASPS),
        (SpriteRow::WorkIconBowTraining, RHWORKICON_BOW_TRAINING),
        (SpriteRow::WorkIconSwordTraining, RHWORKICON_SWORD_TRAINING),
        (SpriteRow::WorkIconRegeneration, RHWORKICON_REGENERATE),
    ]
}

/// Preserve the historical mixed sparse contract: frame_sizes contains only
/// present pictures, while offsets retains every source slot. These vectors
/// participate in simulation-input identity; do not normalize them as cleanup.
fn ground_mark_from_metadata(
    pics: &[Option<PictureOpacityMetadata>],
) -> Option<GroundMarkSpriteData> {
    let first_pic = pics.iter().flatten().next()?;
    let frame_sizes: Vec<(u16, u16)> = pics
        .iter()
        .flatten()
        .map(|pic| (pic.width, pic.height))
        .collect();
    // Fully transparent frames keep the historical raw-size / zero-offset rule.
    let (cw, ch) = first_pic
        .opaque_bounds
        .map(|(_, _, width, height)| (width, height))
        .unwrap_or((first_pic.width, first_pic.height));
    let per_frame_offsets = pics
        .iter()
        .map(|pic| {
            pic.as_ref()
                .and_then(|pic| pic.opaque_bounds)
                .map(|(x, y, _, _)| (x as i16, y as i16))
                .unwrap_or((0, 0))
        })
        .collect();
    Some(GroundMarkSpriteData {
        half_w: cw as f32 * 0.5,
        half_h: ch as f32 * 0.5,
        frame_sizes,
        per_frame_offsets,
    })
}

pub fn ground_mark_sprite_data(
    resources: &mut ResourceManager,
) -> Result<Option<GroundMarkSpriteData>> {
    if !resources.has_resource(RHID_GROUND_FOCUS) {
        return Ok(None);
    }
    let pictures = resources
        .get_picture_opacity_metadata(RHID_GROUND_FOCUS)
        .context("ground marker engine picture metadata")?;
    Ok(ground_mark_from_metadata(&pictures))
}

/// Absent optional resources remain zero. A present but malformed collection
/// must not silently alter deterministic animation lengths.
pub fn titbit_row_frame_counts(resources: &mut ResourceManager) -> Result<Vec<u16>> {
    let mut counts = vec![0; SpriteRow::NumberOfRows as usize];
    for &(row, id) in titbit_sprite_row_resources() {
        if resources.has_resource(id) {
            let count = resources
                .get_nonempty_picture_count(id)
                .with_context(|| format!("titbit resource {id}: frame count"))?;
            counts[row as usize] = u16::try_from(count)
                .with_context(|| format!("titbit resource {id}: frame count exceeds u16"))?;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::{Picture, PixelFormat, SixteenPacking};
    use crate::resource_manager::EncodedPicture;

    #[test]
    fn shipping_metadata_matches_original_without_decoding_resident_frames() {
        // Existing ResourceManager fixture: cjxl0.11.2 2x3 solid red RGB.
        let bytes = vec![
            255, 10, 16, 0, 2, 128, 72, 8, 2, 1, 0, 156, 2, 75, 24, 155, 156, 113, 132, 3, 56, 128,
            3, 56, 32, 74, 192, 57, 5, 1, 0, 32, 68, 128, 8, 16, 1, 34, 64, 228, 255, 145, 123,
            250, 30, 90, 103, 87, 85, 85, 85, 37, 73, 146, 16, 80, 119, 119, 119, 119, 119, 255,
            255, 255, 191, 85, 111, 102, 102, 102, 6, 254, 223, 191, 231, 191, 135, 198, 156, 115,
            174, 181, 207, 189, 73, 146, 36, 4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189,
            175, 187, 187, 187, 27, 254, 223, 191, 231, 191, 135, 198, 156, 115, 174, 181, 207,
            189, 73, 146, 36, 4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189, 175, 187, 187,
            187, 27, 254, 223, 191, 231, 191, 135, 198, 156, 115, 174, 181, 207, 189, 73, 146, 36,
            4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189, 175, 187, 187, 187, 251, 2, 33, 0,
            120, 248, 123, 244, 99, 0, 0,
        ];
        for id in [RHID_GROUND_FOCUS, RHID_ONE_STAR] {
            let (mut original, _) = fixture(id, false);
            let mut shipping = original.clone();
            shipping
                .encode_pictures_for_shipping(|_| {
                    Ok(EncodedPicture {
                        codec: crate::resource_manager::EncodedPictureCodec::JxlRgb565,
                        bytes: bytes.clone(),
                    })
                })
                .unwrap();
            if id == RHID_GROUND_FOCUS {
                let expected = ground_mark_sprite_data(&mut original).unwrap().unwrap();
                let actual = ground_mark_sprite_data(&mut shipping).unwrap().unwrap();
                assert_eq!(
                    (actual.half_w, actual.half_h),
                    (expected.half_w, expected.half_h)
                );
                assert_eq!(actual.frame_sizes, expected.frame_sizes);
                assert_eq!(actual.per_frame_offsets, expected.per_frame_offsets);
            } else {
                assert_eq!(
                    titbit_row_frame_counts(&mut shipping).unwrap(),
                    titbit_row_frame_counts(&mut original).unwrap()
                );
            }
            assert!(
                shipping.pictures_raw(id).is_none(),
                "metadata extraction must not decode JXL frame pixels"
            );
        }
    }

    fn fixture(
        id: i32,
        wrong_type: bool,
    ) -> (
        ResourceManager,
        std::sync::Arc<robin_util::asset_fs::AssetVfs>,
    ) {
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x100u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(if wrong_type { b"TEXT" } else { b"BTTN" });
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        if wrong_type {
            bytes.extend_from_slice(&0u16.to_le_bytes());
        } else {
            // Slots 1 and 3 present; the renderer's source indices remain sparse.
            bytes.extend_from_slice(&0b1010u32.to_le_bytes());
            let picture = Picture {
                width: 2,
                height: 3,
                pitch: 4,
                pixel_format: PixelFormat::Rgb16,
                data: [0, 248].repeat(6),
                palette: None,
            };
            for _ in 0..2 {
                bytes.extend(
                    picture
                        .write_sixteen_to_bytes(SixteenPacking::None)
                        .unwrap(),
                );
            }
        }
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("interface.res", bytes).unwrap();
        let files = std::sync::Arc::new(robin_data_io::sbfile::SbFileSystem::new(vfs.clone()));
        let mut resources = ResourceManager::with_files(files);
        resources.attach_resource_file("interface.res").unwrap();
        (resources, vfs)
    }

    #[test]
    fn missing_optional_resources_and_wrong_type_present_resources_are_distinct() {
        assert!(
            ground_mark_sprite_data(&mut ResourceManager::default())
                .unwrap()
                .is_none()
        );
        assert!(
            titbit_row_frame_counts(&mut ResourceManager::default())
                .unwrap()
                .iter()
                .all(|count| *count == 0)
        );
        let (mut wrong_ground, _) = fixture(RHID_GROUND_FOCUS, true);
        assert!(ground_mark_sprite_data(&mut wrong_ground).is_err());
        let (mut wrong_titbit, _) = fixture(RHID_ONE_STAR, true);
        assert!(titbit_row_frame_counts(&mut wrong_titbit).is_err());
    }

    #[test]
    fn original_sparse_collections_preserve_metadata_and_shared_animation_rows() {
        let (mut ground, _) = fixture(RHID_GROUND_FOCUS, false);
        let data = ground_mark_sprite_data(&mut ground).unwrap().unwrap();
        assert_eq!((data.half_w, data.half_h), (1.0, 1.5));
        assert_eq!(data.frame_sizes, [(2, 3), (2, 3)]);
        assert_eq!(data.per_frame_offsets, [(0, 0); 4]);
        let (mut titbits, _) = fixture(RHID_ONE_STAR, false);
        let counts = titbit_row_frame_counts(&mut titbits).unwrap();
        for row in [SpriteRow::Impact, SpriteRow::OneStar, SpriteRow::Smoke] {
            assert_eq!(counts[row as usize], 2);
        }
        assert_eq!(counts[SpriteRow::Water as usize], 0);
        assert_eq!(titbit_sprite_row_resources().len(), 37);
        let unique: std::collections::BTreeSet<_> = titbit_sprite_row_resources()
            .iter()
            .map(|(row, _)| *row as usize)
            .collect();
        assert_eq!(unique.len(), 37);
    }

    #[test]
    fn failed_original_recovery_and_bad_shipping_headers_do_not_become_empty_metadata() {
        for id in [RHID_GROUND_FOCUS, RHID_ONE_STAR] {
            let (mut original, vfs) = fixture(id, false);
            original.dismiss_resource(id);
            vfs.install_preloaded_asset("interface.res", vec![0])
                .unwrap();
            let (mut shipping, _) = fixture(id, false);
            shipping
                .encode_pictures_for_shipping(|_| Ok(EncodedPicture::jxl_rgba565_keyed(vec![1])))
                .unwrap();
            for resources in [&mut original, &mut shipping] {
                assert!(resources.has_resource(id));
                if id == RHID_GROUND_FOCUS {
                    assert!(ground_mark_sprite_data(resources).is_err());
                } else {
                    assert!(titbit_row_frame_counts(resources).is_err());
                }
            }
        }
    }

    #[test]
    fn sparse_ground_geometry_keeps_compact_sizes_and_original_offset_slots() {
        let mut pictures = vec![
            None,
            Some(PictureOpacityMetadata {
                width: 8,
                height: 6,
                opaque_bounds: Some((3, 2, 4, 2)),
                hit_mask: None,
            }),
            None,
            Some(PictureOpacityMetadata {
                width: 5,
                height: 4,
                opaque_bounds: None,
                hit_mask: None,
            }),
        ];
        let data = ground_mark_from_metadata(&pictures).unwrap();
        assert_eq!((data.half_w, data.half_h), (2.0, 1.0));
        assert_eq!(data.frame_sizes, [(8, 6), (5, 4)]);
        assert_eq!(data.per_frame_offsets, [(0, 0), (3, 2), (0, 0), (0, 0)]);
        pictures[1].as_mut().unwrap().opaque_bounds = None;
        let data = ground_mark_from_metadata(&pictures).unwrap();
        assert_eq!((data.half_w, data.half_h), (4.0, 3.0));
        assert_eq!(data.per_frame_offsets, [(0, 0); 4]);
        assert!(ground_mark_from_metadata(&[None, None]).is_none());
    }

    #[test]
    fn authored_large_offsets_keep_existing_signed_projection() {
        let data = ground_mark_from_metadata(&[Some(PictureOpacityMetadata {
            width: u16::MAX,
            height: 1,
            opaque_bounds: Some((40000, 0, 1, 1)),
            hit_mask: None,
        })])
        .unwrap();
        assert_eq!(data.per_frame_offsets, [(40000u16 as i16, 0)]);
    }
}
