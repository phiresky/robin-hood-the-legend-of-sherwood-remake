use robin_test_support::original_data;

use super::{
    AudioFormat, AudioKind, InterfaceImageFormat, add_character_action_rhs_profiles,
    animation_rhs_paths, animation_rhs_rel_existing, detect_locale_data_dirs,
    detect_official_web_content_edition, exclamation_dat_filename, find_data_dir,
    insert_shipping_audio, insert_standalone_audio, is_common_audio_member, lcid_to_iso,
    level_asset_rel_existing, normalize_robin_profile_index, positional_pair_map,
    prepare_shipping_payload, sxt_is_sres, transcode_audio_to_opus, transcode_sxt_drop_bzip,
    validate_web_content_edition, walk_and_bundle_locale, write_shipping_dependency,
};
use robin_assets::picture::{Picture, PixelFormat, SixteenPacking};
use robin_assets::shipping_datadir::{ShippingAudioAsset, ShippingLocale, ShippingMission};
use robin_engine::profiles::{Action, CharacterProfile, ProfileManager};
use robin_rs::multiplayer::content_identity::WebContentEdition;
use std::fs;

#[test]
fn walkers_preserve_distinct_collection_and_bundle_filters() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested/a.CFG"), b"configuration").unwrap();
    fs::write(temp.path().join("extra.bin"), b"large asset").unwrap();
    let mut files = Vec::new();
    super::collect_files_recursive(temp.path(), &mut files).unwrap();
    files.sort();
    assert_eq!(
        files,
        [
            temp.path().join("extra.bin"),
            temp.path().join("nested/a.CFG")
        ]
    );
    let mut boot = super::ShippingDatadir::default();
    super::walk_and_bundle_small(
        &mut boot,
        temp.path(),
        temp.path(),
        &["cfg"],
        InterfaceImageFormat::Raw,
    )
    .unwrap();
    assert_eq!(boot.raw.len(), 1);
    assert_eq!(boot.raw["nested/a.cfg"], b"configuration");
    let mut locale = ShippingLocale::default();
    walk_and_bundle_locale(
        &mut locale,
        temp.path(),
        temp.path(),
        InterfaceImageFormat::Raw,
    )
    .unwrap();
    assert_eq!(locale.raw.len(), 2);
    assert_eq!(locale.raw["extra.bin"], b"large asset");
}

#[test]
fn optional_directories_preserve_absent_and_non_directory_policy() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file");
    fs::write(&file, []).unwrap();
    assert!(super::optional_directory(temp.path()).unwrap());
    for path in [
        temp.path().join("missing"),
        file.clone(),
        file.join("child"),
    ] {
        assert!(!super::optional_directory(&path).unwrap());
    }
}

#[test]
#[cfg(unix)]
fn walkers_continue_following_file_and_directory_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    // Genuine non-files remain excluded by the collector/locale policy;
    // the small bundle retains its extension filter.
    let _socket = std::os::unix::net::UnixListener::bind(root.join("socket.tmp")).unwrap();
    fs::write(outside.join("source.cfg"), b"linked bytes").unwrap();
    std::os::unix::fs::symlink(outside.join("source.cfg"), root.join("file.cfg")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("directory")).unwrap();
    let mut files = Vec::new();
    super::collect_files_recursive(&root, &mut files).unwrap();
    files.sort();
    assert_eq!(
        files,
        [root.join("directory/source.cfg"), root.join("file.cfg")]
    );
    let mut boot = super::ShippingDatadir::default();
    super::walk_and_bundle_small(&mut boot, &root, &root, &["cfg"], InterfaceImageFormat::Raw)
        .unwrap();
    let mut locale = ShippingLocale::default();
    walk_and_bundle_locale(&mut locale, &root, &root, InterfaceImageFormat::Raw).unwrap();
    for key in ["file.cfg", "directory/source.cfg"] {
        assert_eq!(boot.raw[key], b"linked bytes");
        assert_eq!(locale.raw[key], b"linked bytes");
    }
}

#[test]
#[cfg(unix)]
fn walkers_report_broken_targets_and_optional_roots_report_other_errors() {
    let temp = tempfile::tempdir().unwrap();
    let broken = temp.path().join("broken.ignored");
    std::os::unix::fs::symlink(temp.path().join("absent"), &broken).unwrap();
    // Even an excluded extension cannot conceal a failed entry inspection.
    let results = [
        super::collect_files_recursive(temp.path(), &mut Vec::new()),
        super::walk_and_bundle_small(
            &mut super::ShippingDatadir::default(),
            temp.path(),
            temp.path(),
            &["cfg"],
            InterfaceImageFormat::Raw,
        ),
        walk_and_bundle_locale(
            &mut ShippingLocale::default(),
            temp.path(),
            temp.path(),
            InterfaceImageFormat::Raw,
        ),
    ];
    for result in results {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains(&broken.display().to_string())
        );
    }
    let looping = temp.path().join("loop");
    std::os::unix::fs::symlink(&looping, &looping).unwrap();
    assert!(
        super::optional_directory(&looping)
            .unwrap_err()
            .to_string()
            .contains(&looping.display().to_string())
    );
}

#[test]
fn discovery_level_pair_preserves_resolution_and_parser_error_context() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("Data");
    fs::create_dir_all(data.join("Levels")).unwrap();
    let converter = super::Converter::new(data.clone(), temp.path().join("output"));
    assert_eq!(
        converter
            .parse_level("proto", "mission")
            .unwrap_err()
            .to_string(),
        "proto missing: proto"
    );
    let proto = data.join("Levels/proto.rhp");
    fs::write(&proto, b"malformed proto").unwrap();
    assert_eq!(
        converter
            .parse_level("proto", "mission")
            .unwrap_err()
            .to_string(),
        "mission missing: mission"
    );
    let mission = data.join("Levels/mission.rhm");
    fs::write(&mission, b"malformed mission").unwrap();
    let direct =
        super::parse_level_pair(&proto, &mission, &converter.beggar_civ_indices).unwrap_err();
    let discovery = converter.parse_level("proto", "mission").unwrap_err();
    assert_eq!(
        discovery.root_cause().to_string(),
        direct.root_cause().to_string()
    );
    let context = discovery.to_string();
    assert!(context.contains("parse level proto/mission"), "{context}");
    assert!(context.contains(&proto.display().to_string()), "{context}");
    assert!(
        context.contains(&mission.display().to_string()),
        "{context}"
    );
}

#[test]
fn level_animation_rhs_paths_follow_runtime_ambiance_lookup() {
    let paths = animation_rhs_paths("chariot02").collect::<Vec<_>>();

    assert!(paths.contains(&"Animations/Day/chariot02.rhs".to_owned()));
    assert!(paths.contains(&"Animations/chariot02.rhs".to_owned()));
    assert!(paths.iter().all(|path| !path.starts_with("Characters/")));
}

#[test]
fn shipping_animation_ambiance_uses_original_bit_values() {
    let exists = |path: &str| Some(path.into());

    assert_eq!(
        animation_rhs_rel_existing(1, "river", &exists),
        "Animations/Day/river.rhs"
    );
    assert_eq!(
        animation_rhs_rel_existing(2, "river", &exists),
        "Animations/Fog/river.rhs"
    );
    assert_eq!(
        animation_rhs_rel_existing(4, "river", &exists),
        "Animations/Night/river.rhs"
    );
    assert_eq!(
        animation_rhs_rel_existing(8, "river", &exists),
        "Animations/Day/river.rhs"
    );
}

#[test]
fn shipping_level_assets_follow_exact_ambiance_day_root_lookup() {
    let existing = [
        "Levels/Attack/castle.map",
        "Levels/Day/castle.min",
        "Levels/root.map",
        "Levels/root.min",
    ];
    let exists = |path: &str| existing.contains(&path).then(|| path.into());

    assert_eq!(
        level_asset_rel_existing(8, "castle", ".map", &exists).unwrap(),
        "Levels/Attack/castle.map"
    );
    assert_eq!(
        level_asset_rel_existing(8, "castle", ".min", &exists).unwrap(),
        "Levels/Day/castle.min"
    );
    assert_eq!(
        level_asset_rel_existing(128, "root", ".map", &exists).unwrap(),
        "Levels/root.map"
    );
    assert!(level_asset_rel_existing(16, "missing", ".min", &exists).is_err());
}

#[test]
fn robin_profile_normalization_uses_forest_flag() {
    let profiles = ProfileManager {
        characters: vec![
            CharacterProfile {
                filename: "RobinHood".into(),
                ..CharacterProfile::default()
            },
            CharacterProfile {
                filename: "RobinTown".into(),
                ..CharacterProfile::default()
            },
            CharacterProfile {
                filename: "LittleJohn".into(),
                ..CharacterProfile::default()
            },
        ],
        ..ProfileManager::new()
    };
    assert_eq!(
        normalize_robin_profile_index(&profiles, 1, true).unwrap(),
        0
    );
    assert_eq!(
        normalize_robin_profile_index(&profiles, 0, false).unwrap(),
        1
    );
    assert_eq!(
        normalize_robin_profile_index(&profiles, 2, true).unwrap(),
        2
    );
}

#[test]
fn positional_pairing_drops_conflicting_variant_frames() {
    // Frame 10 pairs consistently with 20; frame 11 pairs with both 21
    // and 22 (a duplicated variant frame against different hub frames)
    // and must be dropped; frame 12 duplicates a consistent pair.
    let pairs = positional_pair_map(&[10, 11, 12, 11, 12], &[20, 21, 30, 22, 30]);
    assert_eq!(pairs.get(&10), Some(&20));
    assert_eq!(pairs.get(&11), None);
    assert_eq!(pairs.get(&12), Some(&30));
}

#[test]
fn exclamation_id_maps_to_original_actor_table_name() {
    assert_eq!(
        exclamation_dat_filename(u32::from_le_bytes(*b"PCRH")),
        "actorPCRH.dat"
    );
}

#[test]
fn character_actions_add_projectile_and_pickup_rhs_capabilities() {
    let mut required = std::collections::BTreeMap::new();
    add_character_action_rhs_profiles(
        &mut required,
        [Action::Bow, Action::Purse, Action::WaspNest],
    );
    for path in [
        "Characters/ACCESSORIES_Arrow.rhs",
        "Characters/BONUS_Arrows.rhs",
        "Characters/ACCESSORIES_MoneyBag.rhs",
        "Characters/ACCESSORIES_Coin.rhs",
        "Characters/BONUS_MoneyBag.rhs",
        "Characters/ACCESSORIES_Wasp.rhs",
        "Characters/ACCESSORIES_WaspSting.rhs",
        "Characters/BONUS_WaspsNest.rhs",
    ] {
        assert!(required.contains_key(path), "missing {path}");
    }
    assert!(!required.contains_key("Characters/RELIC_Crown.rhs"));
}

#[test]
fn resume_reuses_only_an_exact_decoded_payload() {
    let temp = tempfile::tempdir().unwrap();
    let mut payload = ShippingMission::default();
    payload.raw.insert("one.bin".into(), vec![1, 2, 3]);
    let (filename, compressed) =
        prepare_shipping_payload(temp.path(), "Example", &payload, 30, false).unwrap();
    std::fs::write(temp.path().join(&filename), compressed.unwrap()).unwrap();

    let (reused_filename, compressed) =
        prepare_shipping_payload(temp.path(), "Example", &payload, 30, true).unwrap();
    assert_eq!(reused_filename, filename);
    assert!(compressed.is_none());

    let (_, compressed) =
        prepare_shipping_payload(temp.path(), "Example", &payload, 29, true).unwrap();
    assert!(
        compressed.is_some(),
        "a different zstd window must not reuse"
    );

    payload.raw.insert("two.bin".into(), vec![4]);
    let (_, compressed) =
        prepare_shipping_payload(temp.path(), "Example", &payload, 30, true).unwrap();
    assert!(compressed.is_some());
}

#[test]
fn standalone_opus_is_cataloged_without_entering_mission_payload() {
    let temp = tempfile::tempdir().unwrap();
    let assets_dir = temp.path().join("audio/assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let mut catalog = std::collections::BTreeMap::new();
    let payload = ShippingMission::default();
    let opus = b"OggS-fake-OpusHead-test-payload";

    insert_standalone_audio(
        &mut catalog,
        &assets_dir,
        "common",
        "Data/Sounds/Arrow.wav",
        opus,
        1_234,
    )
    .unwrap();

    assert!(payload.raw.is_empty());
    assert!(payload.audio_durations_ms.is_empty());
    let asset = catalog.get("sounds/arrow.opus").unwrap();
    assert_eq!(asset.encoded_size, opus.len() as u32);
    assert_eq!(asset.duration_ms, 1_234);
    assert_eq!(std::fs::read(temp.path().join(&asset.file)).unwrap(), opus);
    assert!(
        !robin_assets::shipping_datadir::encode_mission_native(&payload)
            .windows(opus.len())
            .any(|window| window == opus)
    );
}

#[test]
fn common_audio_excludes_menu_exclamations_and_mission_dialogue() {
    let dialogue = std::collections::BTreeSet::from(["sounds/dialog/line.wav".into()]);
    assert!(is_common_audio_member("arrow_hit.wav", &dialogue));
    assert!(!is_common_audio_member("snd_001.wav", &dialogue));
    assert!(!is_common_audio_member("menu/click.wav", &dialogue));
    assert!(!is_common_audio_member(
        "exclamations/robin/alert.wav",
        &dialogue
    ));
    assert!(!is_common_audio_member("dialog/line.wav", &dialogue));
}

#[test]
fn opus_payload_retains_exact_catalog_membership_without_encoded_bytes() {
    let sample_rate = 8_000u32;
    let sample_count = 800u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + sample_count * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(sample_count * 2).to_le_bytes());
    wav.resize(wav.len() + (sample_count * 2) as usize, 0);

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("arrow.wav");
    std::fs::write(&source, wav).unwrap();
    let mut payload = ShippingMission::default();
    let mut catalog = std::collections::BTreeMap::from([(
        "sounds/arrow.opus".into(),
        ShippingAudioAsset {
            file: "audio/assets/existing.opus".into(),
            encoded_size: 42,
            duration_ms: 100,
            bundle_offset: None,
        },
    )]);

    insert_shipping_audio(
        &mut payload,
        &mut catalog,
        &temp.path().join("audio/assets"),
        "common",
        "Sounds/Arrow.wav",
        &source,
        AudioKind::Effect,
        AudioFormat::Opus,
    )
    .unwrap();

    assert!(payload.raw.is_empty());
    assert_eq!(payload.audio_durations_ms["sounds/arrow.opus"], 100);
}

#[test]
fn opus_membership_only_dependency_is_written_and_decodes() {
    let temp = tempfile::tempdir().unwrap();
    let mut payload = ShippingMission::default();
    payload
        .audio_durations_ms
        .insert("sounds/arrow.opus".into(), 100);

    let relative =
        write_shipping_dependency(temp.path(), "metadata-only-audio", &payload, 30, false)
            .unwrap()
            .expect("Opus membership metadata is a real dependency");
    let filename = std::path::Path::new(&relative)
        .file_name()
        .expect("dependency path has a file name");
    let compressed = std::fs::read(temp.path().join(filename)).unwrap();
    let decoded = robin_assets::shipping_datadir::decode_mission_compressed(&compressed)
        .expect("decode metadata-only dependency");

    assert!(decoded.raw.is_empty());
    assert_eq!(decoded.audio_durations_ms["sounds/arrow.opus"], 100);
}

#[test]
#[ignore = "requires ffmpeg with libopus"]
fn opus_transcode_is_byte_deterministic() {
    let sample_rate = 8_000u32;
    let sample_count = 800u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + sample_count * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(sample_count * 2).to_le_bytes());
    wav.resize(wav.len() + (sample_count * 2) as usize, 0);

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("determinism-fixture.wav");
    std::fs::write(&source, wav).unwrap();
    let first = transcode_audio_to_opus(&source, AudioKind::Voice).unwrap();
    let second = transcode_audio_to_opus(&source, AudioKind::Voice).unwrap();

    assert_eq!(first, second);
    assert!(first.starts_with(b"OggS"));
    assert!(first.windows(8).any(|window| window == b"OpusHead"));
    assert!(
        first
            .windows(b"robinhood-web-shipping".len())
            .any(|window| window == b"robinhood-web-shipping")
    );
}

#[test]
fn locale_detection_preserves_every_installed_pack() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("Data");
    fs::create_dir(&base).unwrap();
    for lcid in ["1033", "1031", "1036", "2047"] {
        fs::create_dir_all(temp.path().join(lcid).join("Data")).unwrap();
    }

    let detected = detect_locale_data_dirs(&base);
    let identities = detected
        .iter()
        .map(|source| (source.lcid, source.iso))
        .collect::<Vec<_>>();
    assert_eq!(identities[0], ("1033", "en-US"));
    assert!(identities.contains(&("1031", "de-DE")));
    assert!(identities.contains(&("1036", "fr-FR")));
    assert!(identities.contains(&("2047", "und")));
    assert_eq!(identities.len(), 4);
    assert_eq!(lcid_to_iso("2047"), "und");
}

#[test]
fn sxt_dispatches_standalone_sixteen_picture_by_content() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("Start.sxt");
    let pixels = vec![0x00, 0x00, 0x1f, 0x00, 0xe0, 0x07, 0xff, 0xff];
    let picture = Picture {
        width: 2,
        height: 2,
        pitch: 4,
        pixel_format: PixelFormat::Rgb16,
        data: pixels.clone(),
        palette: None,
    };
    fs::write(
        &path,
        picture
            .write_sixteen_to_bytes(SixteenPacking::Bzip)
            .unwrap(),
    )
    .unwrap();

    assert!(!sxt_is_sres(&path).unwrap());
    let converted = transcode_sxt_drop_bzip(&path).unwrap();
    assert_eq!(u32::from_le_bytes(converted[4..8].try_into().unwrap()), 0);
    let decoded = Picture::load_sixteen_from_bytes(&converted).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 2));
    assert_eq!(decoded.data, pixels);
}

#[test]
fn sxt_recognizes_sres_resource_container_magic() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("strings.sxt");
    fs::write(&path, b"SRES\0\x01\0\0\0\0\0\0").unwrap();

    assert!(sxt_is_sres(&path).unwrap());
}

#[test]
#[ignore = "requires licensed Leicester demo via ROBINHOOD_DATA_DIR, including English 1033 locale"]
fn authentic_demo_start_sxt_is_a_sixteen_picture() {
    let root = original_data::data_directory("");
    let english = super::discovery::resolve_locale_data_dir(&root, "1033").unwrap_or_else(|| {
        panic!(
            "required English 1033/Data locale is missing from {}",
            root.display()
        )
    });
    let path = super::resolve_data_file(&english, "Interface/Start.sxt").unwrap_or_else(|| {
        panic!(
            "required Interface/Start.sxt is missing from {}",
            english.display()
        )
    });

    assert!(!sxt_is_sres(&path).unwrap());
    let converted = transcode_sxt_drop_bzip(&path).unwrap();
    let decoded = Picture::load_sixteen_from_bytes(&converted).unwrap();
    assert_eq!((decoded.width, decoded.height), (1024, 768));
    assert_eq!(decoded.data.len(), 1024 * 768 * 2);
    assert_eq!(u32::from_le_bytes(converted[4..8].try_into().unwrap()), 0);
}

#[test]
fn locale_start_sxt_is_bundled_as_a_standalone_picture() {
    let temp = tempfile::tempdir().unwrap();
    let interface = temp.path().join("Interface");
    fs::create_dir(&interface).unwrap();
    let picture = Picture {
        width: 2,
        height: 1,
        pitch: 4,
        pixel_format: PixelFormat::Rgb16,
        data: vec![0x34, 0x12, 0x78, 0x56],
        palette: None,
    };
    fs::write(
        interface.join("Start.sxt"),
        picture
            .write_sixteen_to_bytes(SixteenPacking::Bzip)
            .unwrap(),
    )
    .unwrap();

    let mut locale = ShippingLocale::default();
    walk_and_bundle_locale(
        &mut locale,
        temp.path(),
        temp.path(),
        InterfaceImageFormat::Raw,
    )
    .unwrap();

    assert!(locale.res_files.is_empty());
    let bundled = &locale.raw["interface/start.sxt"];
    let decoded = Picture::load_sixteen_from_bytes(bundled).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(decoded.data, picture.data);
    assert_eq!(u32::from_le_bytes(bundled[4..8].try_into().unwrap()), 0);
}

#[test]
fn mixed_official_source_markers_are_rejected_as_ambiguous() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("Data");
    fs::create_dir_all(data.join("Levels")).unwrap();
    fs::write(data.join("Levels/Dem_Lei_MP.rhm"), b"demo marker").unwrap();
    fs::write(data.join("Levels/Sherwood.rhm"), b"full marker").unwrap();

    let error = detect_official_web_content_edition(&data).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("ambiguous"));
    assert!(message.contains("Dem_Lei_MP.rhm"));
    assert!(message.contains("Sherwood.rhm"));
}

#[test]
#[ignore = "requires licensed Leicester demo via ROBINHOOD_DATA_DIR"]
fn authentic_demo_root_has_exact_typed_edition() {
    assert_original_edition(WebContentEdition::Demo);
}

#[test]
#[ignore = "requires licensed full game via ROBINHOOD_DATA_DIR"]
fn authentic_fullgame_root_has_exact_typed_edition() {
    assert_original_edition(WebContentEdition::Full);
}

fn assert_original_edition(expected: WebContentEdition) {
    let root = original_data::data_directory("");
    let data = find_data_dir(&root).unwrap();
    assert_eq!(
        detect_official_web_content_edition(&data).unwrap(),
        expected,
        "wrong edition for {}",
        root.display()
    );
    assert_eq!(
        validate_web_content_edition(&data, expected).unwrap(),
        expected
    );
    let opposite = match expected {
        WebContentEdition::Demo => WebContentEdition::Full,
        WebContentEdition::Full => WebContentEdition::Demo,
    };
    assert!(validate_web_content_edition(&data, opposite).is_err());
}
