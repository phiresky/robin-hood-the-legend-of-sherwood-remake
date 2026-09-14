//! AVIF container fixtures and browser-decode cache tests.
//!
//! Native tests cannot decode AVIF pixels (the web runtime uses the
//! browser's decoder), so each test injects the RGBA a browser decode
//! produces. The cache is process-global: every test holds [`LOCK`] and
//! (re)injects the entries it relies on.

use super::*;
use crate::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16};

const LOSSLESS_8X4: &[u8] = include_bytes!("../testdata/avif/rgba8x4_lossless.avif");
const KEYED_8X4: &[u8] = include_bytes!("../testdata/avif/rgba8x4_keyed_q60.avif");
const RGB_2X3: &[u8] = include_bytes!("../testdata/avif/rgb2x3_q60.avif");

/// Serialize on the crate-wide cache lock and start from an empty cache, so
/// no test observes entries (e.g. boot-scoped ones) another test left behind.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    let guard = TEST_CACHE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clear_scope(ImageScope::Boot).unwrap();
    clear_scope(ImageScope::Mission).unwrap();
    guard
}

/// The keyed fixture's class layout: columns 0-1 transparent, (7,3) shadow,
/// everything else opaque with a colour that is never a key.
fn keyed_pixels() -> DecodedRgba {
    let mut rgba = Vec::with_capacity(8 * 4 * 4);
    for y in 0..4u8 {
        for x in 0..8u8 {
            let alpha = if x < 2 {
                0
            } else if (x, y) == (7, 3) {
                128
            } else {
                255
            };
            rgba.extend_from_slice(&[x * 32, y * 64, 128, alpha]);
        }
    }
    DecodedRgba {
        width: 8,
        height: 4,
        rgba,
    }
}

fn solid_red_2x3() -> DecodedRgba {
    DecodedRgba {
        width: 2,
        height: 3,
        rgba: [255, 0, 0, 255].repeat(6),
    }
}

fn words(picture: &Picture) -> Vec<u16> {
    picture
        .data
        .chunks_exact(2)
        .map(|word| u16::from_le_bytes([word[0], word[1]]))
        .collect()
}

/// Native rav1d decode must equal the pinned `avifdec` (libavif 1.4.2 +
/// libaom 3.15.0 + libyuv) byte for byte — the same pixels Chrome and
/// Firefox produced for these encodes. Covers lossless and lossy keyed
/// RGBA (alpha class markers 0/128/255), opaque RGB, and a real 64x64
/// interface picture for broad colour coverage.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_decode_matches_avifdec_reference_pixels() {
    let _guard = lock();
    for (name, bytes, reference) in [
        (
            "rgba8x4_lossless",
            LOSSLESS_8X4,
            &include_bytes!("../testdata/avif/rgba8x4_lossless.avifdec.rgba")[..],
        ),
        (
            "rgba8x4_keyed_q60",
            KEYED_8X4,
            &include_bytes!("../testdata/avif/rgba8x4_keyed_q60.avifdec.rgba")[..],
        ),
        (
            "rgb2x3_q60",
            RGB_2X3,
            &include_bytes!("../testdata/avif/rgb2x3_q60.avifdec.rgba")[..],
        ),
        (
            "keyed_medium_q60",
            include_bytes!("../testdata/avif/keyed_medium_q60.avif"),
            &include_bytes!("../testdata/avif/keyed_medium_q60.avifdec.rgba")[..],
        ),
    ] {
        let decoded = decode_avif_rgba8(bytes).unwrap_or_else(|error| panic!("{name}: {error:#}"));
        let info = avif_info(bytes).unwrap();
        assert_eq!(
            (decoded.width, decoded.height),
            (u32::from(info.width), u32::from(info.height)),
            "{name}"
        );
        assert_eq!(decoded.rgba.len(), reference.len(), "{name}");
        let first_difference = decoded
            .rgba
            .iter()
            .zip(reference)
            .position(|(actual, expected)| actual != expected);
        assert_eq!(
            first_difference, None,
            "{name}: native decode differs from avifdec at byte {first_difference:?}"
        );
    }
    // The lossless fixture's alpha is exactly the class markers.
    let lossless = decode_avif_rgba8(LOSSLESS_8X4).unwrap();
    for (index, pixel) in lossless.rgba.chunks_exact(4).enumerate() {
        let expected = keyed_pixels().rgba[index * 4 + 3];
        assert_eq!(pixel[3], expected, "lossless alpha at pixel {index}");
    }
}

/// The tracked `multi-team-demos` terrain ships as AVIF under its `.map`
/// name; native builds must sniff and fully decode it (rav1d).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn tracked_mod_terrain_decodes_natively_from_its_map_name() {
    let _guard = lock();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../mods/multi-team-demos/Data/Levels/Day/OpenBattlefield.map");
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(is_avif(&bytes), "tracked mod terrain must be AVIF");
    assert!(!avif_info(&bytes).unwrap().has_alpha);
    assert_eq!(Picture::terrain_dimensions(&bytes).unwrap(), (2508, 2508));
    let terrain = Picture::load_terrain_from_bytes(&bytes).unwrap();
    assert_eq!((terrain.width, terrain.height), (2508, 2508));
    assert_eq!(terrain.data.len(), 2508 * 2508 * 2);
    let first = &terrain.data[..2];
    assert!(
        terrain.data.chunks_exact(2).any(|pixel| pixel != first),
        "decoded terrain is a single flat colour"
    );
    // Not predecoded: native decode must not have populated the cache.
    assert!(!is_decoded(&bytes).unwrap());

    // Loose mod files reach the loaders as streams (level_loading_host's
    // disk fallback), not byte slices: the stream path must sniff AVIF too
    // and consume the whole remainder, from any starting position.
    let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
    let files = robin_data_io::sbfile::SbFileSystem::new(vfs.clone());
    for prefix_len in [0usize, 5] {
        let mut prefixed = vec![0xAB; prefix_len];
        prefixed.extend_from_slice(&bytes);
        vfs.install_preloaded_asset("avif-terrain-fixture.map", prefixed)
            .unwrap();
        let mut file = files.open("avif-terrain-fixture.map").unwrap();
        file.skip(prefix_len as i64, 0).unwrap();
        let streamed = Picture::load_terrain_from_stream(&mut file).unwrap();
        assert_eq!((streamed.width, streamed.height), (2508, 2508));
        assert_eq!(streamed.data, terrain.data);
        assert_eq!(file.tell(), file.get_size());
    }

    // An ISOBMFF file without the AVIF brand is refused, not misparsed as
    // legacy Sixteen terrain.
    let mut not_avif = bytes.clone();
    not_avif[8..12].copy_from_slice(b"mif1");
    let brands_end = u32::from_be_bytes(not_avif[0..4].try_into().unwrap()) as usize;
    for brand in not_avif[16..brands_end].chunks_exact_mut(4) {
        if brand == b"avif" {
            brand.copy_from_slice(b"mif1");
        }
    }
    vfs.install_preloaded_asset("avif-terrain-fixture.map", not_avif)
        .unwrap();
    let mut file = files.open("avif-terrain-fixture.map").unwrap();
    let error = Picture::load_terrain_from_stream(&mut file).unwrap_err();
    assert!(
        format!("{error:#}").contains("without the AVIF brand"),
        "{error:#}"
    );
}

#[test]
fn fixtures_expose_dimensions_alpha_and_brand() {
    let _guard = lock();
    for (bytes, expected) in [
        (
            LOSSLESS_8X4,
            AvifInfo {
                width: 8,
                height: 4,
                has_alpha: true,
            },
        ),
        (
            KEYED_8X4,
            AvifInfo {
                width: 8,
                height: 4,
                has_alpha: true,
            },
        ),
        (
            RGB_2X3,
            AvifInfo {
                width: 2,
                height: 3,
                has_alpha: false,
            },
        ),
    ] {
        assert!(is_avif(bytes));
        assert_eq!(avif_info(bytes).unwrap(), expected);
    }
    assert!(avif_info(b"\xff\x0a jxl codestream").is_err());
}

#[test]
fn insert_validates_dimensions_and_buffer_length() {
    let _guard = lock();
    let mut wrong_size = keyed_pixels();
    wrong_size.width = 4;
    assert!(insert_decoded(KEYED_8X4, ImageScope::Mission, wrong_size).is_err());
    let mut short = keyed_pixels();
    short.rgba.pop();
    assert!(insert_decoded(KEYED_8X4, ImageScope::Mission, short).is_err());
    assert!(
        insert_decoded(RGB_2X3, ImageScope::Mission, keyed_pixels()).is_err(),
        "an 8x4 buffer must not be accepted for a 2x3 image"
    );
}

#[test]
fn undecoded_images_error_on_the_web_and_decode_natively_without_caching() {
    let _guard = lock();
    // A trailing byte after the last ISOBMFF box keeps the container and
    // the AV1 payload intact but gives the image a content key nothing ever
    // inserted. (Corrupting the payload itself is not an option natively:
    // rav1d 1.1.0 aborts on some corrupt tile data — see `native_av1`.)
    let mut bytes = KEYED_8X4.to_vec();
    bytes.push(0);
    assert_eq!(avif_info(&bytes).unwrap(), avif_info(KEYED_8X4).unwrap());
    assert!(!is_decoded(&bytes).unwrap());
    #[cfg(target_arch = "wasm32")]
    {
        let error = format!("{:#}", decoded_rgba(&bytes).unwrap_err());
        assert!(error.contains("8x4"), "{error}");
        assert!(error.contains("sha256"), "{error}");
        assert!(Picture::load_minimap_from_bytes(&bytes).is_err());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let decoded = decoded_rgba(&bytes).unwrap();
        assert_eq!((decoded.width, decoded.height), (8, 4));
        assert_eq!(Picture::load_minimap_from_bytes(&bytes).unwrap().width, 8);
        assert!(
            !is_decoded(&bytes).unwrap(),
            "native decode must not populate the cache"
        );
    }
}

#[test]
fn keyed_and_opaque_loaders_reconstruct_classes_from_injected_pixels() {
    let _guard = lock();
    insert_decoded(KEYED_8X4, ImageScope::Mission, keyed_pixels()).unwrap();
    insert_decoded(RGB_2X3, ImageScope::Mission, solid_red_2x3()).unwrap();

    let minimap = Picture::load_minimap_from_bytes(KEYED_8X4).unwrap();
    assert_eq!((minimap.width, minimap.height, minimap.pitch), (8, 4, 16));
    for (index, pixel) in words(&minimap).into_iter().enumerate() {
        match (index % 8, index / 8) {
            (0 | 1, _) => assert_eq!(pixel, TRANSPARENT_COLOR_16, "pixel {index}"),
            (7, 3) => assert_eq!(pixel, SHADOW_KEY, "pixel {index}"),
            _ => assert!(
                pixel != TRANSPARENT_COLOR_16 && pixel != SHADOW_KEY,
                "opaque pixel {index} decoded as key {pixel:#06x}"
            ),
        }
    }
    assert_eq!(Picture::terrain_dimensions(KEYED_8X4).unwrap(), (8, 4));
    // Keyed images never go through the opaque terrain decoder. An AVIF
    // without an alpha item (libavif omits it when every pixel is opaque)
    // decodes through the keyed loader as all-opaque, never as a key.
    assert!(Picture::load_terrain_from_bytes(KEYED_8X4).is_err());
    let all_opaque = Picture::load_minimap_from_bytes(RGB_2X3).unwrap();
    assert!(
        words(&all_opaque)
            .iter()
            .all(|&pixel| pixel != TRANSPARENT_COLOR_16 && pixel != SHADOW_KEY)
    );

    let terrain = Picture::load_terrain_from_bytes(RGB_2X3).unwrap();
    assert_eq!((terrain.width, terrain.height), (2, 3));
    assert!(words(&terrain).iter().all(|&pixel| pixel == 0xF800));
    assert_eq!(
        Picture::load_terrain_from_bytes_parallel(RGB_2X3)
            .unwrap()
            .data,
        terrain.data
    );
    assert_eq!(Picture::terrain_dimensions(RGB_2X3).unwrap(), (2, 3));
}

#[test]
fn retain_mission_images_keeps_boot_and_listed_entries() {
    let _guard = lock();
    insert_decoded(KEYED_8X4, ImageScope::Mission, keyed_pixels()).unwrap();
    insert_decoded(RGB_2X3, ImageScope::Mission, solid_red_2x3()).unwrap();
    retain_mission_images(&[RGB_2X3]).unwrap();
    assert!(is_decoded(RGB_2X3).unwrap());
    assert!(!is_decoded(KEYED_8X4).unwrap());

    // A boot entry survives mission eviction, and a boot re-insert promotes
    // an existing mission entry.
    insert_decoded(KEYED_8X4, ImageScope::Boot, keyed_pixels()).unwrap();
    insert_decoded(RGB_2X3, ImageScope::Boot, solid_red_2x3()).unwrap();
    retain_mission_images(&[]).unwrap();
    assert!(is_decoded(KEYED_8X4).unwrap());
    assert!(is_decoded(RGB_2X3).unwrap());
    clear_scope(ImageScope::Boot).unwrap();
    assert!(!is_decoded(KEYED_8X4).unwrap());
}

#[cfg(feature = "engine-adapters")]
mod shipping {
    use super::*;
    use crate::frame_holder::UNMAPPED_DICT;
    use crate::resource_manager::{EncodedPicture, EncodedPictureCodec};
    use crate::shipping_datadir::{
        RleJxlPlacement, ShippingMissionPayload, ShippingSprite, ShippingSpriteBank,
        SpriteRleJxlChunk,
    };

    #[test]
    fn encoded_picture_avif_and_raw_codecs() {
        let _guard = lock();
        insert_decoded(KEYED_8X4, ImageScope::Boot, keyed_pixels()).unwrap();
        let avif = EncodedPicture::avif_rgba565_keyed(KEYED_8X4.to_vec());
        assert_eq!(avif.dimensions().unwrap(), (8, 4));
        assert_eq!(avif.browser_image_bytes(), Some(KEYED_8X4));
        assert_eq!(
            avif.decode().unwrap().data,
            Picture::load_minimap_from_bytes(KEYED_8X4).unwrap().data
        );

        let source = Picture {
            width: 3,
            height: 2,
            pitch: 6,
            pixel_format: crate::picture::PixelFormat::Rgb16,
            data: [
                TRANSPARENT_COLOR_16,
                SHADOW_KEY,
                0x1234,
                0xFFFF,
                0x0000,
                0xF800,
            ]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect(),
            palette: None,
        };
        let raw = EncodedPicture::rgb565_raw(&source).unwrap();
        assert_eq!(raw.codec, EncodedPictureCodec::Rgb565Raw);
        assert_eq!(raw.browser_image_bytes(), None);
        assert_eq!(raw.dimensions().unwrap(), (3, 2));
        let decoded = raw.decode().unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.pitch), (3, 2, 6));
        assert_eq!(decoded.data, source.data);
        let mut truncated = raw.clone();
        truncated.bytes.pop();
        assert!(truncated.decode().is_err());
    }

    // RLE sprite words shared with `shipping_datadir::tests`: A (4x4) with a
    // shadow literal and a transparent-key literal, B (4x2).
    const RLE_A_WORDS: [u16; 16] = [
        0,
        3,
        0x1234,
        0x5678,
        0x9ABC,
        0xDEF0,
        0xFFFF,
        0xFFFF,
        1,
        2,
        SHADOW_KEY,
        TRANSPARENT_COLOR_16,
        2,
        3,
        0x0000,
        0xFFFF,
    ];
    const RLE_B_WORDS: [u16; 7] = [0, 1, 0x8410, 0x4208, 3, 3, 0xF800];

    #[test]
    fn avif_rle_chunk_materializes_the_exact_source_canvases() {
        use crate::rle_jxl::{canvas_to_rgba, decode_rle_canvas};
        let _guard = lock();
        // The browser decode of a LOSSLESS atlas: A at (0,0), B at (4,0),
        // class-marked alpha, gutters transparent.
        let (canvas_a, _) = decode_rle_canvas(4, 4, &RLE_A_WORDS).unwrap();
        let (canvas_b, _) = decode_rle_canvas(4, 2, &RLE_B_WORDS).unwrap();
        let mut atlas = vec![0u8; 8 * 4 * 4];
        for (canvas, width, height, x0) in [(&canvas_a, 4, 4, 0), (&canvas_b, 4, 2, 4)] {
            let rgba = canvas_to_rgba(canvas).unwrap();
            for y in 0..height {
                let dst = (y * 8 + x0) * 4;
                atlas[dst..dst + width * 4]
                    .copy_from_slice(&rgba[y * width * 4..(y + 1) * width * 4]);
            }
        }
        insert_decoded(
            LOSSLESS_8X4,
            ImageScope::Mission,
            DecodedRgba {
                width: 8,
                height: 4,
                rgba: atlas,
            },
        )
        .unwrap();

        let sprite = |width: u16, height: u16| ShippingSprite {
            width,
            height,
            dictionary_index: UNMAPPED_DICT,
            packed_data: std::sync::Arc::new(Vec::new()),
            raster: None,
        };
        let chunk = SpriteRleJxlChunk {
            rhs: "Animations/Day/avif.rhs".into(),
            jxl_blobs: vec![LOSSLESS_8X4.to_vec()],
            sprite_ids: vec![5, 9],
            placements: vec![
                RleJxlPlacement {
                    blob: 0,
                    x: 0,
                    y: 0,
                },
                RleJxlPlacement {
                    blob: 0,
                    x: 4,
                    y: 0,
                },
            ],
        };
        let payload = ShippingMissionPayload {
            sprite_bank: Some(ShippingSpriteBank {
                signature: 7,
                dictionaries: Vec::new(),
                sprite_count: 16,
                sprites: vec![(5, sprite(4, 4)), (9, sprite(4, 2))],
                vq_chunks: Vec::new(),
                rle_jxl_chunks: vec![chunk],
            }),
            raw: [("levels/day/avif.map".to_owned(), RGB_2X3.to_vec())].into(),
            ..ShippingMissionPayload::default()
        };
        assert_eq!(payload.browser_image_blobs().len(), 2);

        let mut bank = payload.sprite_bank.clone().unwrap();
        bank.materialize_rle_jxl_chunks().unwrap();
        for (id, words, width, height) in [
            (5u32, &RLE_A_WORDS[..], 4usize, 4usize),
            (9, &RLE_B_WORDS[..], 4, 2),
        ] {
            let raster = bank
                .sprites
                .iter()
                .find(|(sprite_id, _)| *sprite_id == id)
                .and_then(|(_, sprite)| sprite.raster.clone())
                .unwrap();
            let (expected, _) = decode_rle_canvas(width, height, words).unwrap();
            let actual: Vec<u16> = (0..height)
                .flat_map(|y| raster.row(y, width).unwrap().iter().copied())
                .collect();
            assert_eq!(actual, expected, "sprite {id}");
        }
    }
}
