use super::*;

#[test]
fn grayscale_normalization_reuses_owned_storage_and_preserves_rounding() {
    for data in [
        vec![0],
        vec![255; 16],
        vec![7, 19, 21, 30],
        (0..=255).collect(),
        (100..=200).rev().collect(),
    ] {
        let min = *data.iter().min().unwrap();
        let max = *data.iter().max().unwrap();
        let factor = if max > min {
            255.0 / f32::from(max - min)
        } else {
            0.0
        };
        let expected: Vec<_> = data
            .iter()
            .map(|value| (f32::from(*value - min) * factor) as u8)
            .collect();
        let pointer = data.as_ptr();
        let width = data.len() as u32;
        let field = HeightField::normalize_grayscale(data, width, 1);
        assert_eq!(field.data.as_ptr(), pointer);
        assert_eq!(field.data, expected);
        assert_eq!((field.width, field.height), (width, 1));
    }
}

#[test]
fn packed_color_height_fields_preserve_all_pixel_values() {
    let pixels: Vec<u16> = (0..=u16::MAX).collect();
    for is_565 in [true, false] {
        let grayscale: Vec<u8> = pixels
            .iter()
            .map(|&pixel| {
                let (r, g, b) = if is_565 {
                    (
                        ((pixel & 0xf800) >> 8) as u32,
                        ((pixel & 0x07e0) >> 3) as u32,
                        ((pixel & 0x001f) << 3) as u32,
                    )
                } else {
                    (
                        ((pixel & 0x7c00) >> 7) as u32,
                        ((pixel & 0x03e0) >> 2) as u32,
                        ((pixel & 0x001f) << 3) as u32,
                    )
                };
                ((r * 39 + g * 50 + b * 11) / 100) as u8
            })
            .collect();
        let expected = HeightField::from_grayscale(&grayscale, 256, 256);
        let actual = if is_565 {
            HeightField::from_rgb565(&pixels, 256, 256)
        } else {
            HeightField::from_rgb555(&pixels, 256, 256)
        };
        assert_eq!(actual.data, expected.data);
    }
}

#[test]
fn loading_pictures_use_only_the_supplied_preparation_reader() {
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let picture = Picture {
        width: 1,
        height: 1,
        pitch: 2,
        pixel_format: assets_picture::PixelFormat::Rgb16,
        data: vec![0x34, 0x12],
        palette: None,
    };
    let encoded = picture
        .write_sixteen_to_bytes(assets_picture::SixteenPacking::None)
        .unwrap();
    std::fs::write(root.path().join("loading.pak"), encoded.repeat(3)).unwrap();
    let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
    assert_eq!(files.set_primary_path(root.path().to_str().unwrap()), 0);
    let pictures = load_loose_loading_pictures(&files, "loading.pak").unwrap();
    assert!(pictures.iter().all(|loaded| loaded.data == picture.data));
    let isolated = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
    assert!(load_loose_loading_pictures(&isolated, "loading.pak").is_err());
}

// -- HeightField ---------------------------------------------------------

#[test]
fn height_field_from_grayscale_normalizes() {
    // Input range 100..200 should be normalized to 0..255
    let data: Vec<u8> = (100..=200).collect();
    let hf = HeightField::from_grayscale(&data, 101, 1);

    assert_eq!(hf.data[0], 0, "min should map to 0");
    assert_eq!(hf.data[100], 255, "max should map to 255");
    assert_eq!(hf.width, 101);
    assert_eq!(hf.height, 1);
}

#[test]
fn height_field_from_grayscale_uniform_image() {
    // All same value => all zeros (range is 0, normalizer is 0)
    let data = vec![128u8; 16];
    let hf = HeightField::from_grayscale(&data, 4, 4);
    assert!(hf.data.iter().all(|&v| v == 0));
}

#[test]
fn height_field_from_grayscale_full_range() {
    // Already 0..255 => should stay the same after normalization
    let data: Vec<u8> = (0..=255).collect();
    let hf = HeightField::from_grayscale(&data, 256, 1);
    assert_eq!(hf.data[0], 0);
    assert_eq!(hf.data[255], 255);
}

#[test]
fn height_field_from_rgb_luminance() {
    // Pure red (255,0,0) => luminance = 255*39/100 = 99
    // Pure green (0,255,0) => luminance = 255*50/100 = 127
    // Pure blue (0,0,255) => luminance = 255*11/100 = 28
    let rgb = vec![255, 0, 0, 0, 255, 0, 0, 0, 255];
    let hf = HeightField::from_rgb(&rgb, 3, 1);

    // After normalization: min=28, max=127, range=99
    // red:   (99-28)/99*255 ≈ 183
    // green: (127-28)/99*255 = 255
    // blue:  (28-28)/99*255 = 0
    assert_eq!(hf.data[2], 0, "blue should be lowest");
    assert_eq!(hf.data[1], 255, "green should be highest");
    assert!(
        hf.data[0] > 100 && hf.data[0] < 200,
        "red should be mid-high"
    );
}

#[test]
fn height_field_from_rgb565() {
    // White pixel in RGB565: all bits set = 0xFFFF
    // R = (0xFFFF & 0xF800) >> 8 = 0xF8 = 248
    // G = (0xFFFF & 0x07E0) >> 3 = 0xFC = 252
    // B = (0xFFFF & 0x001F) << 3 = 0xF8 = 248
    // Luminance = (248*39 + 252*50 + 248*11)/100 = 249
    //
    // Black pixel = 0x0000 => luminance = 0
    let pixels = vec![0xFFFF_u16, 0x0000];
    let hf = HeightField::from_rgb565(&pixels, 2, 1);

    assert_eq!(hf.data[0], 255, "white should normalize to 255");
    assert_eq!(hf.data[1], 0, "black should normalize to 0");
}

#[test]
fn height_field_from_rgb555() {
    // White in RGB555: 0x7FFF
    // R = (0x7FFF & 0x7C00) >> 7 = 0xF8 = 248
    // G = (0x7FFF & 0x03E0) >> 2 = 0xF8 = 248
    // B = (0x7FFF & 0x001F) << 3 = 0xF8 = 248
    // Luminance = (248*39 + 248*50 + 248*11)/100 = 248
    let pixels = vec![0x7FFF_u16, 0x0000];
    let hf = HeightField::from_rgb555(&pixels, 2, 1);

    assert_eq!(hf.data[0], 255, "white should normalize to 255");
    assert_eq!(hf.data[1], 0, "black should normalize to 0");
}

#[test]
#[should_panic(expected = "grayscale data length")]
fn height_field_from_grayscale_wrong_size_panics() {
    HeightField::from_grayscale(&[0, 1, 2], 2, 2);
}

#[test]
#[should_panic(expected = "RGB data length")]
fn height_field_from_rgb_wrong_size_panics() {
    HeightField::from_rgb(&[0; 5], 2, 1);
}

#[test]
#[should_panic(expected = "height field dimensions must be nonzero")]
fn grayscale_height_field_rejects_zero_width() {
    HeightField::from_grayscale(&[], 0, 1);
}

#[test]
#[should_panic(expected = "height field dimensions must be nonzero")]
fn rgb_height_field_rejects_zero_height() {
    HeightField::from_rgb(&[], 1, 0);
}

#[test]
#[should_panic(expected = "height field dimensions must be nonzero")]
fn rgb565_height_field_rejects_empty_dimensions() {
    HeightField::from_rgb565(&[], 0, 0);
}

#[test]
#[should_panic(expected = "height field dimensions must be nonzero")]
fn rgb555_height_field_rejects_zero_width() {
    HeightField::from_rgb555(&[], 0, 10);
}

#[test]
#[should_panic(expected = "height field size overflow")]
fn rgb_height_field_rejects_size_overflow_before_reading_pixels() {
    HeightField::from_rgb(&[], u32::MAX, u32::MAX);
}

// -- HeightField threshold -----------------------------------------------

#[test]
fn threshold_at_zero_progress() {
    assert_eq!(HeightField::compute_threshold(0.0), 256);
}

#[test]
fn threshold_at_full_progress() {
    assert_eq!(HeightField::compute_threshold(1.0), 0);
}

#[test]
fn threshold_at_half_progress() {
    // (1 - 0.5)^2 * 256 = 0.25 * 256 = 64
    assert_eq!(HeightField::compute_threshold(0.5), 64);
}

#[test]
fn threshold_clamped_beyond_bounds() {
    assert_eq!(HeightField::compute_threshold(-1.0), 256);
    assert_eq!(HeightField::compute_threshold(2.0), 0);
}

#[test]
fn threshold_is_monotonically_decreasing() {
    let thresholds: Vec<u32> = (0..=100)
        .map(|i| HeightField::compute_threshold(i as f32 / 100.0))
        .collect();
    for window in thresholds.windows(2) {
        assert!(
            window[0] >= window[1],
            "threshold must decrease with progress"
        );
    }
}

// -- HeightField mask ----------------------------------------------------

#[test]
fn compute_mask_all_initial() {
    let hf = HeightField {
        data: vec![0, 50, 100, 200],
        width: 2,
        height: 2,
    };
    // Threshold 255: no pixel height exceeds it
    let mask = hf.compute_mask(255);
    assert!(mask.iter().all(|&v| !v));
}

#[test]
fn compute_mask_all_final() {
    let hf = HeightField {
        data: vec![10, 50, 100, 200],
        width: 2,
        height: 2,
    };
    // Threshold 0: all heights > 0 except the first test...
    // Actually height 10 > 0 is true
    let mask = hf.compute_mask(0);
    assert!(mask.iter().all(|&v| v));
}

#[test]
fn compute_mask_mixed() {
    let hf = HeightField {
        data: vec![10, 50, 100, 200],
        width: 2,
        height: 2,
    };
    let mask = hf.compute_mask(50);
    assert_eq!(mask, vec![false, false, true, true]);
}

#[test]
fn version_text_uses_packaged_application_version_for_full_game() {
    let text = loading_version_text(LoadingDatadirKind::FullGame);
    assert_eq!(text, crate::version::version_label());
}

#[test]
fn version_text_appends_demo_kind_for_demo_datadirs() {
    assert!(
        loading_version_text(LoadingDatadirKind::DemoI).ends_with(" DEMO I"),
        "Demo I version label should include datadir kind"
    );
    assert!(
        loading_version_text(LoadingDatadirKind::DemoII).ends_with(" DEMO II"),
        "Demo II version label should include datadir kind"
    );
}

// -- LoadingScreen state machine -----------------------------------------

#[test]
fn loading_screen_default_is_inactive() {
    let screen = LoadingScreen::default();
    assert!(!screen.is_active());
    assert_eq!(screen.progress(), 0.0);
}

#[test]
fn loading_screen_initialize_activates() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    assert!(screen.is_active());
    assert_eq!(screen.max_level, 10.0);
    assert_eq!(screen.current_level, 0.0);
    assert_eq!(screen.screen_width, 800);
    assert_eq!(screen.screen_height, 600);
}

#[test]
fn loading_screen_update_sets_level_and_string() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);

    screen.update(42, 5.0);
    assert_eq!(screen.string_id, 42);
    assert_eq!(screen.current_level, 5.0);
    assert_eq!(screen.progress(), 0.5);
}

#[test]
fn loading_screen_update_level_keeps_string() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    screen.update(42, 3.0);

    screen.update_level(7.0);
    assert_eq!(screen.string_id, 42);
    assert_eq!(screen.current_level, 7.0);
}

#[test]
fn loading_screen_absolute_updates_are_monotonic() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    screen.update(42, 7.0);

    screen.update(43, 3.0);
    assert_eq!(screen.string_id, 43);
    assert_eq!(screen.current_level, 7.0);
    assert_eq!(screen.progress(), 0.7);

    screen.update_level(-5.0);
    assert_eq!(screen.current_level, 7.0);
    assert_eq!(screen.progress(), 0.7);
}

#[test]
fn loading_screen_increment_adds_delta() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);

    screen.increment(1, 3.0);
    assert_eq!(screen.current_level, 3.0);
    assert_eq!(screen.string_id, 1);

    screen.increment_level(2.0);
    assert_eq!(screen.current_level, 5.0);
    assert_eq!(screen.string_id, 1); // unchanged
}

#[test]
fn loading_screen_progress_clamped() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);

    screen.update_level(15.0); // exceeds max
    assert_eq!(screen.progress(), 1.0);
}

#[test]
fn loading_screen_progress_zero_max() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 0.0);
    assert_eq!(screen.progress(), 0.0);
}

#[test]
fn loading_screen_sand_threshold_tracks_progress() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);

    // At start: threshold should be max (256)
    assert_eq!(screen.sand_threshold(), 256);

    // At halfway: (1-0.5)^2 * 256 = 64
    screen.update_level(5.0);
    assert_eq!(screen.sand_threshold(), 64);

    // At end: threshold should be 0
    screen.update_level(10.0);
    assert_eq!(screen.sand_threshold(), 0);
}

#[test]
fn loading_screen_close_deactivates() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    screen.close();
    assert!(!screen.is_active());
}

#[test]
fn loading_screen_reinitialize_resets() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    screen.update(5, 8.0);
    screen.close();

    screen.initialize(1024, 768, 20.0);
    assert!(screen.is_active());
    assert_eq!(screen.max_level, 20.0);
    assert_eq!(screen.current_level, 0.0);
    assert_eq!(screen.string_id, 0);
    assert_eq!(screen.screen_width, 1024);
}

// -- Serde round-trip ----------------------------------------------------

#[test]
fn loading_screen_serde_roundtrip() {
    let mut screen = LoadingScreen::default();
    screen.initialize(800, 600, 10.0);
    screen.update(42, 5.0);

    let json = serde_json::to_string(&screen).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({
            "max_level": 10.0, "current_level": 5.0, "string_id": 42,
            "status_text": null, "active": true, "screen_width": 800, "screen_height": 600
        })
    );
    let restored: LoadingScreen = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.max_level, screen.max_level);
    assert_eq!(restored.current_level, screen.current_level);
    assert_eq!(restored.string_id, screen.string_id);
    assert_eq!(restored.active, screen.active);
    assert_eq!(restored.screen_width, screen.screen_width);
}

#[test]
fn height_field_serde_roundtrip() {
    let hf = HeightField::from_grayscale(&[0, 64, 128, 255], 2, 2);
    let json = serde_json::to_string(&hf).unwrap();
    let restored: HeightField = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.data, hf.data);
    assert_eq!(restored.width, hf.width);
    assert_eq!(restored.height, hf.height);
}

// -- get_data_file -------------------------------------------------------

#[test]
fn get_data_file_uses_mission_specific_if_exists() {
    let dir = tempfile::tempdir().unwrap();
    let level_dir = dir.path().join("levels");
    let ambience_dir = level_dir.join("03");
    std::fs::create_dir_all(&ambience_dir).unwrap();

    let mission_file = ambience_dir.join("castle.pak");
    std::fs::write(&mission_file, b"fake").unwrap();

    let result = get_data_file(level_dir.to_str().unwrap(), "/nonexistent", "castle", 3);
    assert_eq!(result, mission_file);
}

#[test]
fn get_data_file_falls_back_to_generic() {
    let dir = tempfile::tempdir().unwrap();
    let iface_dir = dir.path().join("interface");
    std::fs::create_dir_all(&iface_dir).unwrap();

    let loading_file = iface_dir.join("Loading.pak");
    std::fs::write(&loading_file, b"fake").unwrap();

    let result = get_data_file(
        "/nonexistent/levels",
        iface_dir.to_str().unwrap(),
        "castle",
        3,
    );
    assert_eq!(result, loading_file);
}

#[test]
#[should_panic(expected = "unable to find data file")]
fn get_data_file_panics_when_nothing_found() {
    get_data_file("/nonexistent/levels", "/nonexistent/iface", "castle", 3);
}

// -- format_version_string -----------------------------------------------

#[test]
fn format_version_string_basic() {
    assert_eq!(format_version_string(1, 2, "Gold"), "v1.2 Gold");
}

#[test]
fn format_version_string_empty_release() {
    assert_eq!(format_version_string(2, 0, ""), "v2.0 ");
}

// -- Artwork placement on the logical canvas ------------------------------

#[test]
fn loading_art_rect_fills_matching_canvas() {
    assert_eq!(
        loading_art_rect(640, 480, 640, 480),
        Rect {
            x: 0,
            y: 0,
            w: 640,
            h: 480
        }
    );
}

#[test]
fn loading_art_rect_centres_on_widescreen_canvas() {
    // 853x480 is the bounded-widescreen logical canvas for a 16:9 window
    // at the Low preset: the 4:3 artwork keeps its height and sits in the
    // middle instead of hugging the left edge.
    assert_eq!(
        loading_art_rect(640, 480, 853, 480),
        Rect {
            x: 107,
            y: 0,
            w: 640,
            h: 480
        }
    );
    // A taller canvas than the artwork scales the pictures up to fit.
    assert_eq!(
        loading_art_rect(640, 480, 1280, 720),
        Rect {
            x: 160,
            y: 0,
            w: 960,
            h: 720
        }
    );
}
