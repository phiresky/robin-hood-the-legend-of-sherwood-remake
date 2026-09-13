
use super::*;
use robin_util::asset_fs::{AssetVfs, Bundle};

#[test]
fn shared_manifest_loads_preserve_source_and_distinguish_missing_from_corrupt() {
    let vfs = AssetVfs::new();
    let root = Path::new("shared-manifest-fixture");
    let path = root.join("datadir.bin");
    assert!(try_load_from(&vfs, root).unwrap().is_none());
    assert!(ShippingDatadir::load_from_vfs(&vfs, &path).is_err());

    let mut fixture = ShippingDatadir::default();
    fixture.raw.insert("fixture.bin".to_owned(), vec![7, 8, 9]);
    let compressed = zstd_compress_with_window(&encode_native(&fixture), 30).unwrap();
    vfs.install_preloaded_asset(path.to_str().unwrap(), compressed)
        .unwrap();

    let required = ShippingDatadir::load_from_vfs(&vfs, &path).unwrap();
    let optional = try_load_from(&vfs, root).unwrap().unwrap();
    for loaded in [&required, &optional] {
        assert_eq!(loaded.raw["fixture.bin"], [7, 8, 9]);
        assert_eq!(
            loaded.source_file_path("part.bin").unwrap(),
            root.join("part.bin")
        );
    }

    vfs.install_preloaded_asset(path.to_str().unwrap(), b"not zstd".to_vec())
        .unwrap();
    for error in [
        ShippingDatadir::load_from_vfs(&vfs, &path).unwrap_err(),
        try_load_from(&vfs, root).unwrap_err(),
    ] {
        assert!(
            error
                .to_string()
                .contains(&format!("decode {}", path.display()))
        );
    }
    assert_eq!(required.raw["fixture.bin"], [7, 8, 9]);
}

fn install_fixture(datadir: ShippingDatadir) -> ShippingDatadir {
    let ShippingAssets { datadir, .. } =
        ShippingAssets::install(Arc::new(datadir), Arc::new(AssetVfs::new())).unwrap();
    Arc::try_unwrap(datadir).unwrap()
}

#[test]
fn captured_locale_keeps_pak_and_descriptor_policy_after_selection_changes() {
    let mut datadir = ShippingDatadir::default();
    datadir
        .pak_files
        .insert("interface/title.pak".into(), vec![]);
    datadir
        .pak_files
        .insert("interface/missing.pak".into(), vec![]);
    let mut shared = LevelDescriptors::default();
    shared.custom_short_briefings.push(Some("Base text".into()));
    datadir.red_files.insert("RHLevelSB.red".into(), shared);
    let mut locale = ShippingLocale::default();
    locale.pak_files.insert(
        "interface/title.pak".into(),
        vec![EncodedPicture::jxl_rgba565_keyed(vec![1])],
    );
    datadir.locales.insert("de-DE".into(), locale);
    let datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("de-DE")).unwrap();
    let captured = datadir.active_locale();
    datadir.set_active_locale(None).unwrap();

    assert_eq!(
        datadir
            .localized_pak_for_locale("Data/Interface/Title.pak", captured)
            .unwrap()
            .len(),
        1
    );
    assert!(
        datadir
            .localized_pak_for_locale("Data/Interface/Missing.pak", captured)
            .is_none()
    );
    assert!(
        datadir
            .localized_level_descriptors_for_locale("RHLevelSB.red", captured)
            .is_none()
    );
    // Subsequent independent lookups see the newly selected base assets.
    assert_eq!(
        datadir
            .localized_pak("Data/Interface/Title.pak")
            .unwrap()
            .len(),
        0
    );
    assert!(
        datadir
            .localized_level_descriptors("RHLevelSB.red")
            .is_some()
    );
}

#[test]
fn valid_selected_locale_keeps_missing_assets_optional() {
    let mut datadir = ShippingDatadir::default();
    datadir
        .locales
        .insert("de-DE".into(), ShippingLocale::default());
    let datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("de-DE")).unwrap();
    assert!(datadir.active_resource("Data/Text/Level.res").is_none());
    assert!(datadir.active_pak("Data/Interface/Missing.pak").is_none());
    assert!(datadir.active_level_descriptors("missing.red").is_none());
    assert!(datadir.active_profiles().is_none());
}

#[test]
#[should_panic(expected = "invalid active shipping locale")]
fn invalid_active_locale_cannot_masquerade_as_missing_resource() {
    let datadir = install_fixture(ShippingDatadir::default());
    // Generic VFS selection can be configured independently of this
    // manifest. Shipping lookup must detect that broken invariant.
    datadir
        .asset_vfs()
        .select_locale(Some("@bad@".into()), None)
        .unwrap();
    datadir.active_resource("Data/Text/Level.res");
}

#[test]
#[should_panic(expected = "is not installed")]
fn uninstalled_active_locale_cannot_fall_back_to_shared_descriptors() {
    let datadir = install_fixture(ShippingDatadir::default());
    datadir
        .asset_vfs()
        .select_locale(Some("de-DE".into()), None)
        .unwrap();
    datadir.localized_level_descriptors("RHLevelSB.red");
}

#[test]
#[ignore = "requires ROBIN_BROWSER_CONTENT_FIXTURE pointing to the retained Demo shipping blob"]
fn retained_demo_descriptor_resolves_actual_localized_popup_and_briefing() {
    let path = std::env::var("ROBIN_BROWSER_CONTENT_FIXTURE")
        .expect("set ROBIN_BROWSER_CONTENT_FIXTURE to the retained Demo shipping blob");
    let datadir = ShippingDatadir::load_from_file(Path::new(&path)).unwrap();
    let datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("1033")).unwrap();
    assert!(datadir.active_level_descriptors("RHLevelSB.red").is_none());
    let descriptor = datadir
        .localized_level_descriptors("RHLevelSB.red")
        .unwrap();
    let mut text = datadir
        .active_resource("Data/Text/Level.res")
        .unwrap()
        .clone();
    assert!(
        !text
            .get_string(descriptor.popup_text.text_table_id, 0)
            .unwrap()
            .is_empty()
    );
    assert!(
        !text
            .get_string(descriptor.short_briefing.text_table_id, 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn shared_descriptor_indices_work_with_demo_locale_overlay() {
    let mut datadir = ShippingDatadir::default();
    let mut descriptor = LevelDescriptors::default();
    descriptor.short_briefing.text_table_id = 123;
    descriptor.popup_text.text_table_id = 456;
    datadir.red_files.insert("RHLevelSB.red".into(), descriptor);
    datadir
        .locales
        .insert("en-US".into(), ShippingLocale::default());
    datadir
        .locales
        .insert("de-DE".into(), ShippingLocale::default());
    let mut datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("1033")).unwrap();
    let resolved = datadir
        .localized_level_descriptors("Data\\Text\\RHLevelSB.red")
        .unwrap();
    assert_eq!(resolved.short_briefing.text_table_id, 123);
    assert_eq!(resolved.popup_text.text_table_id, 456);
    // Shared indices must not make an absent translated string table appear.
    datadir
        .res_files
        .insert("text/level.res".into(), ResourceManager::new());
    datadir.set_active_locale(Some("de-DE")).unwrap();
    assert!(
        datadir
            .localized_level_descriptors("rhlevelsb.red")
            .is_some()
    );
    assert!(datadir.active_resource("Data/Text/Level.res").is_none());
    assert!(datadir.localized_level_descriptors("missing.red").is_none());
}

#[test]
fn localized_descriptor_override_precedes_shared_metadata() {
    let mut datadir = ShippingDatadir::default();
    datadir
        .red_files
        .insert("RHLevelSB.red".into(), LevelDescriptors::default());
    let mut locale = ShippingLocale::default();
    let mut translated = LevelDescriptors::default();
    translated.short_briefing.text_table_id = 789;
    translated
        .custom_short_briefings
        .push(Some("Localized objective".into()));
    locale.red_files.insert("rhlevelsb.red".into(), translated);
    datadir.locales.insert("de-DE".into(), locale);
    let datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("de-DE")).unwrap();
    let resolved = datadir
        .localized_level_descriptors("RHLevelSB.red")
        .unwrap();
    assert_eq!(resolved.short_briefing.text_table_id, 789);
    assert_eq!(
        resolved.custom_short_briefings[0].as_deref(),
        Some("Localized objective")
    );
}

#[test]
#[should_panic(expected = "ambiguous shared level descriptor")]
fn shared_descriptor_rejects_ambiguous_case_aliases() {
    let mut datadir = ShippingDatadir::default();
    datadir
        .red_files
        .insert("RHLevelSB.red".into(), LevelDescriptors::default());
    datadir
        .red_files
        .insert("rhlevelsb.red".into(), LevelDescriptors::default());
    let datadir = install_fixture(datadir);
    datadir.localized_level_descriptors("RHLevelSB.red");
}

#[test]
fn shared_descriptor_authored_strings_do_not_cross_locale_boundary() {
    let mut datadir = ShippingDatadir::default();
    let mut descriptor = LevelDescriptors::default();
    descriptor
        .custom_popup_texts
        .push(Some("Not translated".into()));
    datadir.red_files.insert("rhlevelsb.red".into(), descriptor);
    datadir
        .locales
        .insert("de-DE".into(), ShippingLocale::default());
    let datadir = install_fixture(datadir);
    datadir.set_active_locale(Some("de-DE")).unwrap();
    assert!(
        datadir
            .localized_level_descriptors("RHLevelSB.red")
            .is_none()
    );
    datadir.set_active_locale(None).unwrap();
    assert!(
        datadir
            .localized_level_descriptors("RHLevelSB.red")
            .is_some()
    );
}

#[test]
fn expanded_decode_limit_includes_exact_boundary_and_truncation() {
    let compressed = zstd_max_compress(&vec![7; 4096]).unwrap();
    assert_eq!(
        decompress_shipping_with_limit(&compressed, 4096).unwrap(),
        vec![7; 4096]
    );
    assert!(
        decompress_shipping_with_limit(&compressed, 4095)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    assert!(decompress_shipping_with_limit(&compressed[..compressed.len() / 2], 4096).is_err());
}

#[test]
fn duplicate_preload_error_preserves_authenticated_bytes() {
    let datadir = ShippingDatadir::default();
    datadir
        .cache_preloaded_file("payload".into(), vec![1])
        .unwrap();
    assert!(
        datadir
            .cache_preloaded_file("payload".into(), vec![2])
            .is_err()
    );
    assert_eq!(datadir.preloaded_file("payload").unwrap().as_slice(), &[1]);
}

#[test]
fn mission_replacement_retires_only_its_own_stream_after_success() {
    fn payload(name: &str) -> ShippingMission {
        let level = LoadedLevel::hackable_from_json(
            br#"{
                "map_filename":"test", "spawn":[5,5],
                "walkable_polygon":[[0,0],[100,0],[100,100],[0,100]]
            }"#,
        )
        .unwrap();
        let mut mission = ShippingMission::default();
        mission.levels.insert(name.into(), level);
        mission
    }
    let first = ShippingAssets::install(
        Arc::new(ShippingDatadir::default()),
        Arc::new(AssetVfs::new()),
    )
    .unwrap();
    let second = ShippingAssets::install(
        Arc::new(ShippingDatadir::default()),
        Arc::new(AssetVfs::new()),
    )
    .unwrap();
    first
        .datadir()
        .install_mission("old", payload("old"))
        .unwrap();
    second
        .datadir()
        .install_mission("old", payload("old"))
        .unwrap();
    let retained = first.datadir().loaded_mission("old").unwrap();
    let old = retained.sprite_streaming().publisher(3, 300);
    let independent = second
        .datadir()
        .loaded_mission("old")
        .unwrap()
        .sprite_streaming()
        .publisher(3, 300);
    let mut invalid = payload("bad");
    invalid.raw.insert("../escape".into(), vec![1]);
    assert!(first.datadir().install_mission("bad", invalid).is_err());
    assert!(!old.is_retired());
    assert!(old.publish_chunk(100, &[(7, Arc::new(vec![1]))]));
    first.datadir().activate_mission("old").unwrap();
    assert!(
        !old.is_retired(),
        "same mission restart preserves publication"
    );
    first
        .datadir()
        .install_mission("new", payload("new"))
        .unwrap();
    assert!(old.is_retired());
    assert!(!old.publish_chunk(100, &[(8, Arc::new(vec![2]))]));
    assert!(!independent.is_retired());
    assert!(independent.publish_chunk(100, &[(7, Arc::new(vec![3]))]));
}

#[test]
fn mission_publication_rejects_invalid_raw_before_replacing_selection() {
    let datadir = install_fixture(ShippingDatadir::default());
    let good = ShippingMission::default();
    good.raw_bundle
        .set(Arc::new(BTreeMap::from([("old".into(), vec![1].into())])))
        .unwrap();
    datadir
        .runtime
        .loaded_missions
        .write()
        .unwrap()
        .insert("old".into(), Arc::new(good));
    datadir.activate_mission("old").unwrap();
    let snapshot = datadir.selection_snapshot();
    let bad = ShippingMission::default();
    bad.raw_bundle
        .set(Arc::new(BTreeMap::from([(
            "../escape".into(),
            vec![2].into(),
        )])))
        .unwrap();
    assert!(datadir.publish_mission("new", &bad).is_err());
    assert_eq!(datadir.selection_snapshot().generation, snapshot.generation);
    assert_eq!(datadir.active_mission_name().as_deref(), Some("old"));
    assert!(datadir.active_mission_payload().is_some());
    assert_eq!(datadir.asset_vfs().read("old").unwrap(), [1]);
}

#[test]
fn captured_mission_resources_survive_later_activation() {
    use robin_engine::coordinates::{SpriteAnchor, SpriteSize};
    use robin_engine::sprite_script::{FrameKind, SpriteScriptor};

    let datadir = install_fixture(ShippingDatadir::default());
    for (name, width) in [("first", 12.0), ("second", 24.0)] {
        let mut mission = ShippingMission::default();
        mission.raw_bundle.set(Arc::new(BTreeMap::new())).unwrap();
        mission.payload.rhs_files.insert(
            "Characters/Robin.rhs".into(),
            RhsData {
                signature: 77,
                profiles: vec![(
                    "Robin".into(),
                    SpriteInfo {
                        scripts: Arc::new(vec![]),
                        conversion: Arc::new(vec![]),
                        size: SpriteSize::new(width, 8.0),
                        center: SpriteAnchor::ZERO,
                    },
                )],
            },
        );
        datadir
            .runtime
            .loaded_missions
            .write()
            .unwrap()
            .insert(name.into(), Arc::new(mission));
    }
    datadir.activate_mission("first").unwrap();
    let first = datadir.mission_resource_environment("first").unwrap();
    datadir.activate_mission("second").unwrap();
    let second = datadir.mission_resource_environment("second").unwrap();
    assert!(datadir.mission_resource_environment("first").is_err());
    for (resources, width) in [(first, 12.0), (second, 24.0)] {
        let mut scriptor = SpriteScriptor::with_resources(resources);
        let info = scriptor
            .load(
                "Data/Characters/Robin.rhs",
                "Robin",
                "Robin",
                FrameKind::Character,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(info.size, SpriteSize::new(width, 8.0));
    }
}

#[test]
fn mission_snapshots_pin_matching_parsed_and_raw_payloads() {
    let datadir = Arc::new(install_fixture(ShippingDatadir::default()));
    for name in ["first", "second"] {
        let payload = ShippingMission::default();
        payload
            .raw_bundle
            .set(Arc::new(BTreeMap::from([(
                "marker".into(),
                name.as_bytes().into(),
            )])))
            .unwrap();
        datadir
            .runtime
            .loaded_missions
            .write()
            .unwrap()
            .insert(name.into(), Arc::new(payload));
    }
    datadir.activate_mission("first").unwrap();
    let writer = datadir.clone();
    let handle = std::thread::spawn(move || {
        for _ in 0..50 {
            writer.activate_mission("second").unwrap();
            writer.activate_mission("first").unwrap();
        }
    });
    let competing_writer = datadir.clone();
    let competing = std::thread::spawn(move || {
        for _ in 0..50 {
            competing_writer.activate_mission("first").unwrap();
            competing_writer.activate_mission("second").unwrap();
        }
    });
    for _ in 0..500 {
        let (selection, payload) = datadir.mission_selection_snapshot();
        let payload = payload.unwrap();
        assert!(Arc::ptr_eq(
            selection.active_bundle.as_ref().unwrap(),
            payload.raw_bundle.get().unwrap()
        ));
        assert_eq!(
            selection.active_bundle.as_ref().unwrap()["marker"].as_ref(),
            selection.mission.as_ref().unwrap().as_bytes()
        );
    }
    handle.join().unwrap();
    competing.join().unwrap();
}

#[test]
fn installed_locale_selection_is_isolated_and_failure_preserves_generation() {
    fn install() -> ShippingAssets {
        let mut datadir = ShippingDatadir::default();
        for (name, value) in [("en-US", 1), ("de-DE", 2)] {
            let mut locale = ShippingLocale::default();
            locale.raw.insert("text/fixture".into(), vec![value]);
            datadir.locales.insert(name.into(), locale);
        }
        ShippingAssets::install(Arc::new(datadir), Arc::new(AssetVfs::new())).unwrap()
    }
    let first = install();
    let second = install();
    first.datadir().set_active_locale(Some("en-US")).unwrap();
    second.datadir().set_active_locale(Some("de-DE")).unwrap();
    assert_eq!(first.vfs().read("text/fixture").unwrap(), [1]);
    assert_eq!(second.vfs().read("text/fixture").unwrap(), [2]);
    assert_eq!(
        first.datadir().active_locale_name().as_deref(),
        Some("en-US")
    );
    let old = first.vfs().selection_snapshot();
    assert!(first.datadir().set_active_locale(Some("fr-FR")).is_err());
    assert_eq!(first.vfs().selection_snapshot().generation, old.generation);
    assert_eq!(first.vfs().read("text/fixture").unwrap(), [1]);
}

#[test]
fn native_shipping_format_roundtrips_and_rejects_legacy_payloads() {
    let mut datadir = ShippingDatadir::default();
    datadir.raw.insert("test.bin".into(), vec![1, 2, 3]);
    datadir
        .audio_durations_ms
        .insert("musics/menu.opus".into(), 9_876);
    datadir.audio_assets.insert(
        "sounds/arrow.opus".into(),
        ShippingAudioAsset {
            file: "audio/assets/0123.opus".into(),
            encoded_size: 456,
            duration_ms: 789,
            bundle_offset: None,
        },
    );
    datadir.missions.insert(
        "MissionOne".into(),
        ShippingMissionRef {
            forest_level: true,
            files: vec!["missions/mission-one.rhmission.zst".into()],
        },
    );
    datadir
        .character_rhs_files
        .insert(7, vec!["rhs/character-seven.rhmission.zst".into()]);
    datadir
        .character_audio_files
        .insert(7, vec!["audio/character-seven.rhmission.zst".into()]);
    datadir.character_exclamation_ids.insert(7, 0x5043_5248);
    datadir
        .mission_exclamation_ids
        .insert("MissionOne".into(), vec![0x534F_4C44]);
    datadir.saved_world_rhs_files = vec!["rhs/saved-objects.rhmission.zst".into()];
    let mut german = ShippingLocale {
        source_lcid: Some("1031".into()),
        ..ShippingLocale::default()
    };
    german.raw.insert("text/level.res".into(), vec![7, 8, 9]);
    datadir.locales.insert("de-DE".into(), german);

    let encoded = encode_native(&datadir);
    assert_eq!(&encoded[..8], b"RHDDNA16");
    assert_eq!(&encoded[..8], &SHIPPING_DATADIR_MAGIC);
    let decoded = decode_native(&encoded).expect("decode native shipping datadir");
    assert_eq!(decoded.raw.get("test.bin"), Some(&vec![1, 2, 3]));
    assert_eq!(
        decoded.audio_durations_ms.get("musics/menu.opus"),
        Some(&9_876)
    );
    assert_eq!(
        decoded.audio_assets.get("sounds/arrow.opus"),
        Some(&ShippingAudioAsset {
            file: "audio/assets/0123.opus".into(),
            encoded_size: 456,
            duration_ms: 789,
            bundle_offset: None,
        })
    );
    assert_eq!(
        decoded.mission_ref("MissionOne").unwrap().files,
        vec!["missions/mission-one.rhmission.zst"]
    );
    assert!(decoded.mission_ref("MissionOne").unwrap().forest_level);
    assert_eq!(
        decoded.character_rhs_files.get(&7).unwrap(),
        &["rhs/character-seven.rhmission.zst"]
    );
    assert_eq!(
        decoded.character_audio_files.get(&7).unwrap(),
        &["audio/character-seven.rhmission.zst"]
    );
    assert_eq!(
        decoded.character_exclamation_ids.get(&7),
        Some(&0x5043_5248)
    );
    assert_eq!(
        decoded.mission_exclamation_ids.get("MissionOne").unwrap(),
        &[0x534F_4C44]
    );
    assert_eq!(
        decoded.saved_world_rhs_files,
        ["rhs/saved-objects.rhmission.zst"]
    );
    assert_eq!(
        decoded.locale_raw("1031", "Text/Level.res").unwrap(),
        Some([7, 8, 9].as_slice())
    );

    let mut previous_schema = encoded.clone();
    previous_schema[..8].copy_from_slice(b"RHDDNA13");
    let error = decode_native(&previous_schema).unwrap_err();
    assert!(error.to_string().contains("regenerate datadir.bin"));

    let legacy_unversioned = bitcode::encode(datadir.payload());
    let error = decode_native(&legacy_unversioned).unwrap_err();
    assert!(error.to_string().contains("regenerate datadir.bin"));
}

#[test]
fn canonical_locale_ids_accept_legacy_aliases_without_inventing_identity() {
    assert_eq!(canonical_locale_id("1031").unwrap(), "de-DE");
    assert_eq!(canonical_locale_id("DE_de").unwrap(), "de-DE");
    assert_eq!(canonical_locale_id("zh-hant-tw").unwrap(), "zh-Hant-TW");
    assert_eq!(canonical_locale_id("2047").unwrap(), "und");
    assert_eq!(canonical_locale_id("neutral").unwrap(), "und");
    assert!(canonical_locale_id("../de-DE").is_err());
}

#[test]
fn locale_lookup_and_bundle_use_canonical_keys() {
    let mut locale = ShippingLocale {
        source_lcid: Some("1031".into()),
        ..ShippingLocale::default()
    };
    locale.aliases.insert("1031".into());
    locale.raw.insert("text/dialogue.wav".into(), vec![4, 2]);
    let mut datadir = ShippingDatadir::default();
    datadir.locales.insert("de-DE".into(), locale);

    assert_eq!(
        datadir
            .locale_raw("1031", "Data\\Text\\Dialogue.wav")
            .unwrap(),
        Some([4, 2].as_slice())
    );
    assert!(datadir.locale("fr-FR").unwrap().is_none());
    let first = datadir.locale_bundle("de_de").unwrap().unwrap();
    let second = datadir.locale_bundle("1031").unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        first.get("text/dialogue.wav").map(|bytes| bytes.as_ref()),
        Some([4, 2].as_slice())
    );
}

#[test]
fn locale_bundle_only_uses_english_for_optional_recorded_media() {
    let mut english = ShippingLocale::default();
    english.raw.insert("text/level.res".into(), vec![1]);
    english.raw.insert("interface/start.sxt".into(), vec![2]);
    english
        .raw
        .insert("sounds/exclamations/robin.wav".into(), vec![3]);
    english.raw.insert("cinematics/intro.ogg".into(), vec![4]);

    let mut german = ShippingLocale::default();
    german.raw.insert("text/level.res".into(), vec![5]);

    let mut datadir = ShippingDatadir::default();
    datadir.locales.insert("en-US".into(), english);
    datadir.locales.insert("de-DE".into(), german);

    let bundle = datadir.locale_bundle("de-DE").unwrap().unwrap();
    assert_eq!(
        bundle.get("text/level.res").map(|bytes| bytes.as_ref()),
        Some([5].as_slice())
    );
    assert!(!bundle.contains_key("interface/start.sxt"));
    assert_eq!(
        bundle
            .get("sounds/exclamations/robin.wav")
            .map(|bytes| bytes.as_ref()),
        Some([3].as_slice())
    );
    assert_eq!(
        bundle
            .get("cinematics/intro.ogg")
            .map(|bytes| bytes.as_ref()),
        Some([4].as_slice())
    );
}

#[test]
fn mission_payload_roundtrips_independently() {
    let mut mission = ShippingMission::default();
    mission
        .raw
        .insert("levels/day/map.min".into(), vec![9, 8, 7]);
    mission
        .audio_durations_ms
        .insert("sounds/arrow.opus".into(), 1_234);
    let encoded = encode_mission_native(&mission);
    assert_eq!(&encoded[..8], b"RHMISN08");
    let compressed = zstd_compress_with_window(&encoded, 30).unwrap();
    let decoded = decode_mission_compressed(&compressed).unwrap();
    assert_eq!(decoded.raw.get("levels/day/map.min"), Some(&vec![9, 8, 7]));
    assert_eq!(
        decoded.audio_durations_ms.get("sounds/arrow.opus"),
        Some(&1_234)
    );
}

#[test]
fn mission_parts_merge_disjoint_sprite_slots() {
    let sprite = |value| ShippingSprite {
        width: 1,
        height: 1,
        dictionary_index: 0,
        packed_data: Arc::new(vec![value]),
        raster: None,
    };
    let bank = |sprites| ShippingSpriteBank {
        signature: 42,
        dictionaries: Vec::new(),
        sprite_count: 2,
        sprites,
        vq_chunks: Vec::new(),
        rle_jxl_chunks: Vec::new(),
    };
    let mut merged = ShippingMission::from_payload(ShippingMissionPayload {
        sprite_bank: Some(bank(Vec::new())),
        ..ShippingMissionPayload::default()
    });
    merged
        .merge_from(ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(bank(vec![(0, sprite(10))])),
            ..ShippingMissionPayload::default()
        }))
        .unwrap();
    merged
        .merge_from(ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(bank(vec![(1, sprite(20))])),
            ..ShippingMissionPayload::default()
        }))
        .unwrap();

    let sprites = &merged.payload.sprite_bank.unwrap().sprites;
    assert_eq!(sprites[0].1.packed_data.as_slice(), &[10]);
    assert_eq!(sprites[1].1.packed_data.as_slice(), &[20]);
}

/// Base VQ grid (sprite 0), variant VQ grid (sprite 1), second-variant VQ
/// grid (sprite 3, star-2 coded against sprites 0 AND 1): 8x3 pixels =
/// 2x3 tiles.
const VQ_DIMS: (u16, u16) = (8, 3);
const BASE_GRID: [u16; 6] = [5, 6, 7, 5, 6, 7];
const VARIANT_GRID: [u16; 6] = [5, 6, 7, 5, 9, 7];
const SECOND_VARIANT_GRID: [u16; 6] = [5, 6, 7, 5, 9, 8];
const RLE_WORDS: [u16; 3] = [1, 2, 3];
const VQ_ALPHABET: u16 = 16;

fn vq_test_bank(
    sprites: Vec<(u32, ShippingSprite)>,
    vq_chunks: Vec<SpriteVqChunk>,
) -> ShippingSpriteBank {
    ShippingSpriteBank {
        signature: 77,
        dictionaries: Vec::new(),
        sprite_count: 4,
        sprites,
        vq_chunks,
        rle_jxl_chunks: Vec::new(),
    }
}

fn vq_sprite(packed: Vec<u16>) -> ShippingSprite {
    ShippingSprite {
        width: VQ_DIMS.0,
        height: VQ_DIMS.1,
        dictionary_index: 0,
        packed_data: Arc::new(packed),
        raster: None,
    }
}

#[test]
fn rle_priority_uses_total_bytes_and_preserves_ties() {
    let make = |id, sizes: &[usize]| SpriteRleJxlChunk {
        rhs: "same.rhs".into(),
        sprite_ids: vec![id],
        placements: Vec::new(),
        jxl_blobs: sizes.iter().map(|&size| vec![0; size]).collect(),
    };
    let mut chunks = vec![
        make(1, &[2]),
        make(2, &[3, 4]),
        make(3, &[7]),
        make(4, &[5]),
    ];
    order_rle_chunks_by_size(&mut chunks);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.sprite_ids[0])
            .collect::<Vec<_>>(),
        [2, 3, 4, 1]
    );
}

fn priority_chunk(id: u32, bases: &[u32], bytes: usize) -> SpriteVqChunk {
    SpriteVqChunk {
        rhs: "same.rhs".into(),
        base_rhs: None,
        base2_rhs: String::new(),
        alphabet: 1,
        sprite_ids: vec![id],
        base_ids: bases.iter().copied().map(Some).collect(),
        base2_ids: Vec::new(),
        self_refs: false,
        blob: vec![0; bytes],
    }
}

#[test]
fn downstream_priority_distinguishes_groups_and_uses_longest_path() {
    let mut second = priority_chunk(2, &[0, 0], 30);
    second.base2_ids = vec![Some(1)];
    let chunks = vec![
        priority_chunk(0, &[], 2),
        priority_chunk(1, &[], 3),
        second,
        priority_chunk(3, &[2], 40),
        priority_chunk(4, &[0], 20),
        priority_chunk(5, &[999], 50),
    ];
    assert_eq!(vq_downstream_costs(&chunks), [72, 73, 70, 40, 20, 50]);
    let reversed: Vec<_> = chunks.into_iter().rev().collect();
    assert_eq!(vq_downstream_costs(&reversed), [50, 20, 40, 70, 73, 72]);
}

#[test]
fn downstream_priority_allows_duplicate_providers_and_leaves_validation_to_bank() {
    let chunks = vec![
        priority_chunk(0, &[1], 2),
        priority_chunk(1, &[0], 3),
        priority_chunk(0, &[], 4),
        priority_chunk(2, &[1], 5),
    ];
    let costs = vq_downstream_costs(&chunks);
    assert_eq!(costs.len(), chunks.len());
    assert_eq!(costs[1], 8);
    assert_eq!(costs[3], 5);
}

/// Chunk mission for the family base: sprite 0 coded standalone.
fn base_chunk_mission() -> ShippingMission {
    use crate::sprite_codec::{SpriteGrid, encode_grids};
    let blob = encode_grids(
        VQ_ALPHABET,
        &[SpriteGrid {
            cols: VQ_DIMS.0 / 4,
            rows: VQ_DIMS.1,
            indices: &BASE_GRID,
        }],
        None,
    )
    .unwrap();
    ShippingMission::from_payload(ShippingMissionPayload {
        sprite_bank: Some(vq_test_bank(
            vec![(0, vq_sprite(Vec::new()))],
            vec![SpriteVqChunk {
                rhs: "Characters/Test00.rhs".into(),
                base_rhs: None,
                base2_rhs: String::new(),
                alphabet: VQ_ALPHABET,
                sprite_ids: vec![0],
                base_ids: vec![None],
                base2_ids: Vec::new(),
                self_refs: false,
                blob,
            }],
        )),
        ..ShippingMissionPayload::default()
    })
}

/// Chunk mission for the variant: sprite 1 coded against base sprite 0,
/// plus an RLE sprite 2 that keeps raw packed words.
fn variant_chunk_mission() -> ShippingMission {
    use crate::sprite_codec::{SpriteGrid, encode_grids};
    let blob = encode_grids(
        VQ_ALPHABET,
        &[SpriteGrid {
            cols: VQ_DIMS.0 / 4,
            rows: VQ_DIMS.1,
            indices: &VARIANT_GRID,
        }],
        Some(&[Some(&BASE_GRID)]),
    )
    .unwrap();
    ShippingMission::from_payload(ShippingMissionPayload {
        sprite_bank: Some(vq_test_bank(
            vec![
                (1, vq_sprite(Vec::new())),
                (
                    2,
                    ShippingSprite {
                        width: 4,
                        height: 1,
                        dictionary_index: UNMAPPED_DICT,
                        packed_data: Arc::new(RLE_WORDS.to_vec()),
                        raster: None,
                    },
                ),
            ],
            vec![SpriteVqChunk {
                rhs: "Characters/Test01.rhs".into(),
                base_rhs: Some("Characters/Test00.rhs".into()),
                base2_rhs: String::new(),
                alphabet: VQ_ALPHABET,
                sprite_ids: vec![1],
                base_ids: vec![Some(0)],
                base2_ids: Vec::new(),
                self_refs: false,
                blob,
            }],
        )),
        ..ShippingMissionPayload::default()
    })
}

/// Chunk mission for the third family member: sprite 3 star-2 coded
/// against base sprite 0 AND sibling sprite 1 (both from other chunks).
fn second_variant_chunk_mission() -> ShippingMission {
    use crate::sprite_codec::{SpriteGrid, encode_grids_multi};
    let blob = encode_grids_multi(
        VQ_ALPHABET,
        &[SpriteGrid {
            cols: VQ_DIMS.0 / 4,
            rows: VQ_DIMS.1,
            indices: &SECOND_VARIANT_GRID,
        }],
        Some(&[Some(&BASE_GRID)]),
        Some(&[Some(&VARIANT_GRID)]),
    )
    .unwrap();
    ShippingMission::from_payload(ShippingMissionPayload {
        sprite_bank: Some(vq_test_bank(
            vec![(3, vq_sprite(Vec::new()))],
            vec![SpriteVqChunk {
                rhs: "Characters/Test02.rhs".into(),
                base_rhs: Some("Characters/Test00.rhs".into()),
                base2_rhs: "Characters/Test01.rhs".into(),
                alphabet: VQ_ALPHABET,
                sprite_ids: vec![3],
                base_ids: vec![Some(0)],
                base2_ids: vec![Some(1)],
                self_refs: false,
                blob,
            }],
        )),
        ..ShippingMissionPayload::default()
    })
}

/// Lossless 8x4 RGBA JXL atlas (`cjxl -d 0 --alpha_distance=0 -e 7`)
/// holding two RLE sprites: A (4x4) at (0,0) and B (4x2) at (4,0),
/// generated from the exact canvases of `RLE_A_WORDS` / `RLE_B_WORDS`
/// — opaque pixels expanded 565 -> 888, and every pixel's alpha set to
/// its class marker. Lossless + 565-representable colors means
/// materialization must reproduce the source words bit-for-bit.
const RLE_JXL_FIXTURE: &[u8] = &[
    0xFF, 0x0A, 0x18, 0x70, 0xB0, 0x12, 0x08, 0x00, 0x10, 0x00, 0x18, 0x01, 0x4B, 0x18, 0x93, 0x8E,
    0x83, 0x83, 0x84, 0x13, 0xC4, 0x63, 0x8B, 0xCA, 0x5D, 0x40, 0x16, 0x00, 0x7C, 0x30, 0xE4, 0xEA,
    0xA5, 0xF8, 0xDF, 0x8C, 0x8B, 0x31, 0x02, 0x46, 0xED, 0x77, 0x3F, 0xAA, 0xD1, 0xA2, 0x2F, 0x10,
    0x60, 0x7A, 0x67, 0x49, 0x52, 0x7C, 0x91, 0x51, 0x6C, 0x16, 0x20, 0x7E, 0x31, 0x86, 0x20, 0x46,
    0x21, 0x68, 0xAF, 0x6A, 0x5B, 0xBB, 0x5E, 0x77, 0xA3, 0xC3, 0x95, 0x72, 0xE0, 0xC6, 0x69, 0x1E,
    0xBC, 0x01,
];
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
    crate::frame_holder::SHADOW_KEY,
    crate::frame_holder::TRANSPARENT_COLOR_16,
    2,
    3,
    0x0000,
    0xFFFF,
];
const RLE_B_WORDS: [u16; 7] = [0, 1, 0x8410, 0x4208, 3, 3, 0xF800];

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn rle_jxl_atlas_parallelism_preserves_placement_order_and_errors() {
    let mut chunk = SpriteRleJxlChunk {
        rhs: "Animations/Day/parallel.rhs".into(),
        jxl_blobs: vec![RLE_JXL_FIXTURE.to_vec(); 3],
        sprite_ids: vec![5, 9],
        placements: vec![
            RleJxlPlacement {
                blob: 2,
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
    let dims = [(4, 4), (4, 2)];
    for (threads, parallel) in [(1, true), (4, true), (4, false)] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let rasters = pool
            .install(|| {
                ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(
                    &chunk, &dims, parallel,
                )
            })
            .unwrap();
        assert_eq!(
            rasters.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [5, 9]
        );
        for ((_, raster), ((width, height), words)) in rasters
            .iter()
            .zip(dims.into_iter().zip([&RLE_A_WORDS[..], &RLE_B_WORDS[..]]))
        {
            let (expected, _) =
                crate::rle_jxl::decode_rle_canvas(width as usize, height as usize, words).unwrap();
            let actual: Vec<_> = (0..height as usize)
                .flat_map(|y| raster.row(y, width as usize).unwrap().iter().copied())
                .collect();
            assert_eq!(actual, expected);
        }
    }
    // Even an unreferenced atlas must be validated. Parallel collection
    // must propagate its error rather than silently dropping it.
    chunk.jxl_blobs[1] = vec![0];
    for parallel in [false, true] {
        let error =
            ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(&chunk, &dims, parallel)
                .unwrap_err();
        assert!(format!("{error:#}").contains("RLE-JXL blob 1 of Animations/Day/parallel.rhs"));
    }
}

#[test]
fn rle_jxl_chunks_materialize_exact_words_from_lossless_fixture() {
    use crate::rle_jxl;
    let sprite = |w: u16, h: u16| ShippingSprite {
        width: w,
        height: h,
        dictionary_index: UNMAPPED_DICT,
        packed_data: Arc::new(Vec::new()),
        raster: None,
    };
    let mission = ShippingMission::from_payload(ShippingMissionPayload {
        sprite_bank: Some(ShippingSpriteBank {
            signature: 7,
            dictionaries: Vec::new(),
            sprite_count: 16,
            sprites: vec![(5, sprite(4, 4)), (9, sprite(4, 2))],
            vq_chunks: Vec::new(),
            rle_jxl_chunks: vec![SpriteRleJxlChunk {
                rhs: "Animations/Day/test.rhs".into(),
                jxl_blobs: vec![RLE_JXL_FIXTURE.to_vec()],
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
            }],
        }),
        ..ShippingMissionPayload::default()
    });
    // Ship it the way the converter does, then materialize like a
    // mission install.
    let compressed = zstd_compress_with_window(&encode_mission_native(&mission), 30).unwrap();
    let mut decoded = decode_mission_compressed(&compressed).unwrap();
    let bank = decoded.sprite_bank.as_mut().unwrap();
    bank.materialize_rle_jxl_chunks().unwrap();
    assert!(bank.rle_jxl_chunks.is_empty());
    // Both sprites now window into ONE shared atlas — nothing was
    // copied out of it, and no RLE words were rebuilt.
    let rasters: Vec<_> = [5u32, 9]
        .iter()
        .map(|id| bank.sprite_row(*id).unwrap().raster.clone().unwrap())
        .collect();
    assert!(bank.sprite_row(5).unwrap().packed_data.is_empty());
    assert!(Arc::ptr_eq(&rasters[0].atlas, &rasters[1].atlas));
    assert_eq!((rasters[0].stride, rasters[0].x), (8, 0));
    assert_eq!(rasters[1].x, 4);
    // The raster is exactly the canvas the packed words decompress to:
    // lossless color plus the class-carrying alpha reproduces it.
    for (raster, words, width, height) in [
        (&rasters[0], &RLE_A_WORDS[..], 4usize, 4usize),
        (&rasters[1], &RLE_B_WORDS[..], 4, 2),
    ] {
        let (expected, used) = rle_jxl::decode_rle_canvas(width, height, words).unwrap();
        assert_eq!(used, words.len());
        let actual: Vec<u16> = (0..height)
            .flat_map(|y| raster.row(y, width).unwrap().iter().copied())
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn vq_chunks_roundtrip_and_materialize_in_any_merge_order() {
    // Serialize each chunk exactly the way the converter ships it.
    let reload = |mission: &ShippingMission| {
        let compressed = zstd_compress_with_window(&encode_mission_native(mission), 30).unwrap();
        decode_mission_compressed(&compressed).unwrap()
    };
    // Fetch completion order is nondeterministic on wasm: merge the
    // star-2 chunk first (its base2 sibling itself decodes against the
    // family base), then the variant, then the base, and materialize.
    let mut merged = ShippingMission::default();
    merged
        .merge_part(reload(&second_variant_chunk_mission()))
        .unwrap();
    merged.merge_part(reload(&variant_chunk_mission())).unwrap();
    merged.merge_part(reload(&base_chunk_mission())).unwrap();
    let bank = merged.sprite_bank.as_mut().unwrap();
    bank.materialize_vq_chunks(&BTreeMap::new()).unwrap();

    assert!(bank.vq_chunks.is_empty());
    assert_eq!(
        bank.sprite_row(0).unwrap().packed_data.as_slice(),
        BASE_GRID
    );
    assert_eq!(
        bank.sprite_row(1).unwrap().packed_data.as_slice(),
        VARIANT_GRID
    );
    assert_eq!(
        bank.sprite_row(2).unwrap().packed_data.as_slice(),
        RLE_WORDS
    );
    assert_eq!(
        bank.sprite_row(3).unwrap().packed_data.as_slice(),
        SECOND_VARIANT_GRID
    );
}

#[test]
fn variant_vq_chunk_without_base_chunk_is_an_error() {
    let mut merged = ShippingMission::default();
    merged.merge_part(variant_chunk_mission()).unwrap();
    let error = merged
        .sprite_bank
        .as_mut()
        .unwrap()
        .materialize_vq_chunks(&BTreeMap::new())
        .unwrap_err();
    assert!(
        error.to_string().contains("base sprite 0"),
        "unexpected error: {error}"
    );
}

#[test]
fn star2_vq_chunk_without_base2_chunk_is_an_error() {
    // The base chunk arrives but the base2 sibling chunk never does: the
    // star-2 chunk must fail loudly, naming the missing base2 RHS.
    let mut merged = ShippingMission::default();
    merged.merge_part(second_variant_chunk_mission()).unwrap();
    merged.merge_part(base_chunk_mission()).unwrap();
    let error = merged
        .sprite_bank
        .as_mut()
        .unwrap()
        .materialize_vq_chunks(&BTreeMap::new())
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("base2 sprite 1") && message.contains("Characters/Test01.rhs"),
        "unexpected error: {message}"
    );
}

#[test]
fn shipping_installation_owns_vfs_and_has_first_priority() {
    let vfs = Arc::new(AssetVfs::new());
    let mut loose = Bundle::new();
    loose.insert("shared.dat".to_string(), b"loose".to_vec().into());
    vfs.mount_bundle(Arc::new(loose)).unwrap();

    let mut datadir = ShippingDatadir::default();
    datadir
        .raw
        .insert("shared.dat".to_string(), b"shipping".to_vec());
    datadir
        .raw
        .insert("sounds/menu.opus".to_string(), vec![1, 2, 3, 4]);
    datadir
        .audio_durations_ms
        .insert("sounds/menu.opus".to_string(), 250);
    let installed = ShippingAssets::install(Arc::new(datadir), vfs.clone()).unwrap();

    assert!(Arc::ptr_eq(installed.vfs(), &vfs));
    assert!(installed.datadir().raw.is_empty());
    assert_eq!(
        installed.datadir().raw_asset("shared.dat"),
        Some(&b"shipping"[..])
    );
    assert_eq!(
        installed
            .datadir()
            .active_audio_metadata(Path::new("Data/Sounds/Menu.wav")),
        Some((4, 250))
    );
    assert_eq!(installed.vfs().read("shared.dat").unwrap(), b"shipping");
}

#[test]
fn remote_audio_catalog_resolves_legacy_aliases() {
    let mut datadir = ShippingDatadir::default();
    datadir.set_remote_base_url("https://example.test/build/Data/".into());
    datadir.audio_assets.insert(
        "sounds/arrow.opus".into(),
        ShippingAudioAsset {
            file: "audio/assets/abc.opus".into(),
            encoded_size: 321,
            duration_ms: 654,
            bundle_offset: None,
        },
    );
    datadir.audio_assets.insert(
        "sounds/exclamations/expressions/alert.opus".into(),
        ShippingAudioAsset {
            file: "audio/assets/voice.opus".into(),
            encoded_size: 111,
            duration_ms: 222,
            bundle_offset: None,
        },
    );

    let expected = RemoteAudioAsset {
        url: "https://example.test/build/Data/audio/assets/abc.opus".into(),
        encoded_size: 321,
        duration_ms: 654,
        bundle_offset: None,
    };
    assert_eq!(
        datadir.remote_audio_asset(Path::new("Data/Sounds/Arrow.wav")),
        Some(expected.clone())
    );
    assert_eq!(
        datadir.remote_audio_asset(Path::new("arrow.wav")),
        Some(expected.clone())
    );
    assert_eq!(
        datadir.remote_audio_asset(Path::new("/games/Robin Hood/Data/Sounds/Arrow.ogg")),
        Some(expected)
    );
    assert_eq!(
        datadir
            .remote_audio_asset(Path::new("Expressions/Alert.wav"))
            .unwrap()
            .url,
        "https://example.test/build/Data/audio/assets/voice.opus"
    );
    assert_eq!(
        datadir.active_audio_metadata(Path::new("Data/Sounds/Arrow.wav")),
        Some((321, 654))
    );
}

#[test]
fn audio_warmup_membership_is_exact_for_boot_and_active_mission() {
    let mut datadir = install_fixture(ShippingDatadir::default());
    for key in [
        "sounds/menu/click.opus",
        "sounds/exclamations/robin/alert.opus",
        "sounds/not-mounted.opus",
    ] {
        datadir.audio_assets.insert(
            key.into(),
            ShippingAudioAsset {
                file: format!("audio/assets/{key}"),
                encoded_size: 10,
                duration_ms: 100,
                bundle_offset: None,
            },
        );
    }
    datadir
        .audio_durations_ms
        .insert("sounds/menu/click.opus".into(), 100);
    let mut mission = ShippingMission::default();
    mission
        .audio_durations_ms
        .insert("sounds/exclamations/robin/alert.opus".into(), 100);
    datadir
        .runtime
        .loaded_missions
        .write()
        .unwrap()
        .insert("MissionA".into(), Arc::new(mission));
    datadir
        .asset_vfs()
        .select_mission(Some("MissionA".into()), Arc::new(Default::default()))
        .unwrap();

    assert_eq!(
        datadir.boot_audio_keys(),
        vec!["sounds/menu/click.opus".to_owned()]
    );
    assert_eq!(
        datadir.active_audio_keys(),
        vec!["sounds/exclamations/robin/alert.opus".to_owned()]
    );
}

#[test]
fn shipping_installation_propagates_invalid_bundle_path() {
    let vfs = Arc::new(AssetVfs::new());
    let mut datadir = ShippingDatadir::default();
    datadir
        .raw
        .insert("../escape.dat".to_string(), b"bad".to_vec());

    let datadir = Arc::new(datadir);
    let generation = vfs.selection_snapshot().generation;
    let error = ShippingAssets::install(datadir.clone(), vfs.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("mount shipping raw asset bundle")
    );
    assert!(datadir.runtime.installed.get().is_none());
    assert!(vfs.authority_snapshot().is_empty());
    assert_eq!(vfs.selection_snapshot().generation, generation);
    assert_eq!(datadir.raw_asset("../escape.dat"), Some(&b"bad"[..]));
    // A retained decode can be corrected and retried, not left half-bound.
    let mut datadir = Arc::try_unwrap(datadir).unwrap();
    datadir.raw.remove("../escape.dat");
    datadir.raw.insert("valid.dat".into(), b"good".to_vec());
    let installed = ShippingAssets::install(Arc::new(datadir), vfs).unwrap();
    assert_eq!(installed.vfs().read("valid.dat").unwrap(), b"good");
}

#[test]
#[should_panic(expected = "decoded data has no VFS")]
fn decoded_data_has_no_implicit_runtime_authority() {
    ShippingDatadir::default().asset_vfs();
}

#[test]
fn installation_state_is_excluded_from_shared_payload_wire_bytes() {
    let mut datadir = ShippingDatadir::default();
    datadir.raw.insert("marker".into(), vec![42]);
    let datadir = Arc::new(datadir);
    let native = encode_native(&datadir);
    let json = serde_json::to_vec(&*datadir).unwrap();
    let installed = ShippingAssets::install(datadir.clone(), Arc::new(AssetVfs::new())).unwrap();
    assert_eq!(encode_native(installed.datadir()), native);
    assert_eq!(serde_json::to_vec(&**installed.datadir()).unwrap(), json);
}

#[test]
fn concurrent_installation_publishes_only_one_mount() {
    let mut datadir = ShippingDatadir::default();
    datadir.raw.insert("marker".into(), vec![42]);
    let datadir = Arc::new(datadir);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let datadir = datadir.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    let vfs = Arc::new(AssetVfs::new());
                    let generation = vfs.selection_snapshot().generation;
                    barrier.wait();
                    let result = ShippingAssets::install(datadir, vfs.clone());
                    (vfs, generation, result)
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results
            .iter()
            .filter(|(_, _, result)| result.is_ok())
            .count(),
        1
    );
    for (vfs, generation, result) in results {
        if let Ok(installed) = result {
            assert!(Arc::ptr_eq(datadir.asset_vfs(), installed.vfs()));
            assert_eq!(vfs.read("marker").unwrap(), [42]);
            let before = vfs.selection_snapshot().generation;
            assert!(ShippingAssets::install(datadir.clone(), vfs.clone()).is_err());
            assert_eq!(vfs.selection_snapshot().generation, before);
        } else {
            assert!(vfs.read("marker").is_err());
            assert!(vfs.authority_snapshot().is_empty());
            assert_eq!(vfs.selection_snapshot().generation, generation);
        }
    }
}
