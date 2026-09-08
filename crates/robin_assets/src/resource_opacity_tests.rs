use super::*;
use crate::picture::PixelFormat;
use robin_engine::resource_ids::{RHID_GROUND_FOCUS, RHMAP_CORNER};

fn picture() -> Picture {
    Picture {
        width: 2,
        height: 3,
        pitch: 4,
        pixel_format: PixelFormat::Rgb16,
        data: [0x07c0u16, 0x07c0, 0x001f, 0xf800, 0x07c0, 0x07c0]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect(),
        palette: None,
    }
}

fn header_only(_: &Picture) -> Result<EncodedPicture> {
    // Valid 2x3 RGB JXL image header, without frame pixels. Metadata queries
    // must work while full pixel decode fails (the production stream is RGBA).
    Ok(EncodedPicture {
        codec: EncodedPictureCodec::JxlRgb565,
        bytes: vec![255, 10, 16, 0, 2, 128, 72, 8, 2, 1, 0],
    })
}

#[test]
fn exported_opacity_matches_pixels_without_loading_jxl_frames() {
    let mut manager = ResourceManager::new();
    manager
        .data
        .pictures
        .insert(RHID_GROUND_FOCUS, vec![None, Some(picture())]);
    manager
        .data
        .pictures
        .insert(RHMAP_CORNER, vec![Some(picture()), Some(picture()), None]);
    let before_ground = manager
        .get_picture_opacity_metadata(RHID_GROUND_FOCUS)
        .unwrap();
    let before_corner = manager.get_picture_opacity_metadata(RHMAP_CORNER).unwrap();
    let pic = picture();
    assert_eq!(
        before_ground[1].as_ref().unwrap().opaque_bounds,
        pic.opaque_bounds_16()
    );
    assert_eq!(
        before_ground[1].as_ref().unwrap().opaque_bounds,
        Some((0, 1, 2, 1))
    );
    let words: Vec<_> = pic
        .data
        .chunks_exact(2)
        .map(|px| u16::from_le_bytes([px[0], px[1]]))
        .collect();
    let expected = robin_engine::minimap::HitMask::from_pixels_u16(2, 3, &words, 0x07c0);
    let actual = before_corner[1].clone().unwrap().into_hit_mask().unwrap();
    for y in 0..3 {
        for x in 0..2 {
            assert_eq!(expected.is_opaque(x, y), actual.is_opaque(x, y));
        }
    }
    manager.encode_pictures_for_shipping(header_only).unwrap();
    let mut roundtrip: ResourceManager = bitcode::decode(&bitcode::encode(&manager)).unwrap();
    assert_eq!(
        roundtrip
            .get_picture_opacity_metadata(RHID_GROUND_FOCUS)
            .unwrap(),
        before_ground
    );
    assert_eq!(
        roundtrip
            .get_picture_opacity_metadata(RHMAP_CORNER)
            .unwrap(),
        before_corner
    );
    assert!(roundtrip.pictures_raw(RHMAP_CORNER).is_none());
    assert!(roundtrip.get_pictures(RHMAP_CORNER).is_err());
}

#[test]
fn transparent_empty_frames_and_decoded_replacements_keep_their_geometry() {
    let mut manager = ResourceManager::new();
    let mut transparent = picture();
    transparent.data = [0x07c0u16; 6]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    let empty = Picture {
        width: 0,
        height: 3,
        pitch: 0,
        data: vec![],
        ..picture()
    };
    manager.data.pictures.insert(
        RHID_GROUND_FOCUS,
        vec![None, Some(transparent.clone()), Some(empty)],
    );
    let metadata = manager
        .get_picture_opacity_metadata(RHID_GROUND_FOCUS)
        .unwrap();
    assert!(metadata[0].is_none());
    assert_eq!(metadata[1].as_ref().unwrap().opaque_bounds, None);
    assert_eq!(
        (
            metadata[2].as_ref().unwrap().width,
            metadata[2].as_ref().unwrap().height
        ),
        (0, 3)
    );
    manager
        .data
        .pictures
        .insert(RHID_GROUND_FOCUS, vec![Some(picture())]);
    manager.encode_pictures_for_shipping(header_only).unwrap();
    manager
        .data
        .pictures
        .insert(RHID_GROUND_FOCUS, vec![Some(transparent)]);
    assert_eq!(
        manager
            .get_picture_opacity_metadata(RHID_GROUND_FOCUS)
            .unwrap()[0]
            .as_ref()
            .unwrap()
            .opaque_bounds,
        None
    );
}

#[test]
fn missing_corrupt_or_mismatched_metadata_is_an_error() {
    let mut manager = ResourceManager::new();
    assert!(manager.get_picture_opacity_metadata(RHMAP_CORNER).is_err());
    manager
        .data
        .pictures
        .insert(RHMAP_CORNER, vec![None, Some(picture())]);
    manager.encode_pictures_for_shipping(header_only).unwrap();
    let valid = manager.data.picture_opacity[&RHMAP_CORNER].clone();
    manager.data.picture_opacity.remove(&RHMAP_CORNER);
    assert!(manager.get_picture_opacity_metadata(RHMAP_CORNER).is_err());
    for corrupt in 0..6 {
        let mut slots = valid.clone();
        let metadata = slots[1].as_mut().unwrap();
        match corrupt {
            0 => metadata.width += 1,
            1 => metadata.opaque_bounds = Some((2, 0, 1, 1)),
            2 => metadata.hit_mask = None,
            3 => metadata
                .hit_mask
                .as_mut()
                .unwrap()
                .pop()
                .map(|_| ())
                .unwrap(),
            4 => metadata.opaque_bounds = None,
            _ => slots[0] = slots[1].clone(),
        }
        manager.data.picture_opacity.insert(RHMAP_CORNER, slots);
        assert!(manager.get_picture_opacity_metadata(RHMAP_CORNER).is_err());
    }
    let mut malformed = picture();
    malformed.data.pop();
    assert!(PictureOpacityMetadata::from_picture(&malformed, true).is_err());
    assert!(robin_engine::minimap::HitMask::from_opacity(2, 3, vec![true; 5]).is_err());
}
