use super::*;
use winit::keyboard::KeyCode;

#[test]
fn native_and_browser_loads_share_migration_and_validation() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_str().unwrap();
    let mut legacy = KeyConfigStore::new("obsolete".into());
    legacy.configs.insert(7, ProfileKeyConfig::default());
    for invalid in [false, true] {
        if invalid {
            legacy.configs.get_mut(&7).unwrap().custom.bindings = vec![
                crate::key_config::KeyBinding {
                    action: "ZoomIn".into(),
                    primary_key: None,
                    secondary_key: None,
                }; 2
            ];
        }
        fs::write(
            KeyConfigStore::store_path(directory),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let browser = serde_json::to_string(&BrowserKeyConfigEnvelope {
            schema_version: BROWSER_KEY_CONFIG_SCHEMA_VERSION,
            store: &legacy,
        })
        .unwrap();
        let native = KeyConfigStore::load(directory);
        let browser = decode_browser_key_config_archive(&browser, directory);
        if invalid {
            assert_eq!(native.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(browser.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        } else {
            let native = native.unwrap();
            let browser = browser.unwrap();
            assert_eq!(native.save_directory, directory);
            assert_eq!(browser.save_directory, directory);
            assert_eq!(
                serde_json::to_value(&native).unwrap(),
                serde_json::to_value(&browser).unwrap()
            );
            let profile = native.get(7).unwrap();
            for config in [&profile.active, &profile.custom] {
                assert!(config.get_binding("ToggleCloak").is_some());
            }
        }
    }
}

#[test]
fn browser_archive_preserves_wire_format_and_rejects_invalid_documents() {
    let mut store = KeyConfigStore::new("old-directory".into());
    store
        .entry_or_default(7)
        .active
        .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
    let encoded = encode_browser_key_config_archive(&store).unwrap();
    let legacy = serde_json::to_string(&BrowserKeyConfigEnvelope {
        schema_version: BROWSER_KEY_CONFIG_SCHEMA_VERSION,
        store: store.clone(),
    })
    .unwrap();
    assert_eq!(encoded, legacy);
    let decoded = decode_browser_key_config_archive(&encoded, "selected").unwrap();
    assert_eq!(decoded.save_directory, "selected");
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::to_value(&store).unwrap()
    );
    let document: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    for (field, value) in [
        ("schema_version", serde_json::json!(999)),
        ("unexpected", serde_json::json!(true)),
        ("store", serde_json::json!({})),
    ] {
        let mut corrupt = document.clone();
        corrupt[field] = value;
        assert_eq!(
            decode_browser_key_config_archive(&corrupt.to_string(), "selected")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }
    assert!(decode_browser_key_config_archive("not JSON", "selected").is_err());
    assert!(
        decode_browser_key_config_archive(
            &"x".repeat(BROWSER_KEY_CONFIG_BYTE_LIMIT + 1),
            "selected"
        )
        .is_err()
    );
    for id in 0..=KEY_CONFIG_PROFILE_LIMIT as u32 {
        store.entry_or_default(id);
    }
    assert!(encode_browser_key_config_archive(&store).is_err());
}

#[test]
fn restart_ignores_incomplete_staging_and_preserves_selected_directory() {
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().to_str().unwrap();
    let mut store = KeyConfigStore::new(directory.into());
    store
        .entry_or_default(7)
        .active
        .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
    store.save().unwrap();
    fs::write(
        dir.path().join(".robin-user-store-staging-abandoned"),
        b"{partial",
    )
    .unwrap();
    let loaded = KeyConfigStore::load(directory).unwrap();
    assert_eq!(loaded.save_directory, directory);
    assert_eq!(
        loaded
            .get(7)
            .unwrap()
            .active
            .get_binding("ZoomIn")
            .unwrap()
            .primary_key,
        Some(KeyCode::Backspace)
    );
    loaded.save().unwrap();
    assert_eq!(KeyConfigStore::load(directory).unwrap().configs.len(), 1);
}

#[test]
fn unreadable_archive_is_not_a_missing_store() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("keyconfigs.json")).unwrap();
    assert!(KeyConfigStore::load(dir.path().to_str().unwrap()).is_err());
}

#[test]
fn buffered_native_load_preserves_json_error_classification() {
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().to_str().unwrap();
    let path = KeyConfigStore::store_path(directory);
    let mut store = KeyConfigStore::new(directory.into());
    store
        .entry_or_default(7)
        .active
        .set_binding("自定义", Some(KeyCode::KeyA), None);
    let encoded = serde_json::to_vec(&store).unwrap();
    let mut padded = vec![b' '; 8193];
    padded.extend_from_slice(&encoded);
    fs::write(&path, &padded).unwrap();
    assert_eq!(
        serde_json::to_value(KeyConfigStore::load(directory).unwrap()).unwrap(),
        serde_json::to_value(&store).unwrap()
    );
    let mut trailing = encoded.clone();
    trailing.extend_from_slice(b" {}");
    for invalid in [vec![], b"{".to_vec(), vec![0xff], trailing] {
        fs::write(&path, invalid).unwrap();
        assert_eq!(
            KeyConfigStore::load(directory).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn fresh_seeds_both_slots_with_default_preset() {
    let entry = ProfileKeyConfig::fresh();
    assert_eq!(entry.active.key_type, KeyConfig::default_preset().key_type);
    assert_eq!(entry.custom.key_type, KeyConfig::default_preset().key_type);
    assert_eq!(
        entry.active.bindings.len(),
        KeyConfig::default_preset().bindings.len()
    );
}

#[test]
fn entry_or_default_inserts_once() {
    let mut store = KeyConfigStore::new("/tmp/test".into());
    store
        .entry_or_default(7)
        .active
        .set_binding("ZoomIn", Some(KeyCode::Backspace), None);

    let again = store.entry_or_default(7);
    assert_eq!(
        again.active.get_binding("ZoomIn").unwrap().primary_key,
        Some(KeyCode::Backspace)
    );
    assert_eq!(store.configs.len(), 1);
}

#[test]
fn invalid_save_preserves_the_last_valid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().to_str().unwrap();
    let mut valid = KeyConfigStore::new(directory.into());
    valid.entry_or_default(7);
    valid.save().unwrap();
    let path = dir.path().join("keyconfigs.json");
    let original = fs::read(&path).unwrap();
    for custom in [false, true] {
        for invalid_kind in 0..4 {
            let mut invalid = valid.clone();
            let entry = invalid.entry_or_default(7);
            let config = if custom {
                &mut entry.custom
            } else {
                &mut entry.active
            };
            match invalid_kind {
                0 => config.bindings[0].action.clear(),
                1 => config.bindings[0].action = "x".repeat(KEY_CONFIG_ACTION_BYTE_LIMIT + 1),
                2 => config.bindings.push(config.bindings[0].clone()),
                3 => config
                    .bindings
                    .resize(KEY_CONFIG_BINDING_LIMIT + 1, config.bindings[0].clone()),
                _ => unreachable!(),
            }
            assert_eq!(
                invalid.save().unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }
    let mut invalid = valid.clone();
    for id in 0..=KEY_CONFIG_PROFILE_LIMIT as u32 {
        invalid.entry_or_default(id);
    }
    assert_eq!(
        invalid.save().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        serde_json::to_value(KeyConfigStore::load(directory).unwrap()).unwrap(),
        serde_json::to_value(valid).unwrap()
    );
}

#[test]
fn save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let dir_str = dir.path().to_str().unwrap().to_owned();

    {
        let mut store = KeyConfigStore::new(dir_str.clone());
        let entry = store.entry_or_default(3);
        entry.active.set_binding("ZoomIn", Some(KeyCode::F2), None);
        entry.custom.set_binding("ZoomIn", Some(KeyCode::F3), None);
        store.save().unwrap();
    }

    let loaded = KeyConfigStore::load(&dir_str).unwrap();
    let entry = loaded.get(3).expect("profile 3 should round-trip");
    assert_eq!(
        entry.active.get_binding("ZoomIn").unwrap().primary_key,
        Some(KeyCode::F2)
    );
    assert_eq!(
        entry.custom.get_binding("ZoomIn").unwrap().primary_key,
        Some(KeyCode::F3)
    );
}

#[test]
fn loads_keyconfigs_written_before_type_move() {
    let dir = tempfile::tempdir().unwrap();
    let json = r#"{
        "configs": {
            "41": {
                "active": {
                    "bindings": [{
                        "action": "Crouch",
                        "primary_key": "ShiftLeft",
                        "secondary_key": "ShiftRight"
                    }],
                    "key_type": 1
                },
                "custom": {
                    "bindings": [],
                    "key_type": 2
                }
            }
        }
    }"#;
    std::fs::write(dir.path().join("keyconfigs.json"), json).unwrap();

    let loaded = KeyConfigStore::load(dir.path().to_str().unwrap()).unwrap();
    let entry = loaded.get(41).expect("legacy profile should load");
    assert_eq!(entry.active.key_type, 1);
    assert_eq!(
        entry.active.get_binding("Crouch").unwrap().primary_key,
        Some(KeyCode::ShiftLeft)
    );
    assert_eq!(
        entry.active.get_binding("Crouch").unwrap().secondary_key,
        Some(KeyCode::ShiftRight)
    );
    assert_eq!(
        entry
            .active
            .get_binding("ToggleCloak")
            .expect("legacy active config gains cloak action")
            .primary_key,
        Some(KeyCode::KeyV)
    );
    assert_eq!(
        entry
            .custom
            .get_binding("ToggleCloak")
            .expect("legacy custom config gains cloak action")
            .primary_key,
        Some(KeyCode::KeyV)
    );
    assert_eq!(entry.custom.key_type, 2);
    assert_eq!(
        entry
            .active
            .get_binding("PlanQuickActions")
            .expect("legacy active config is migrated")
            .primary_key,
        None,
        "an unrelated custom Shift binding must not be duplicated"
    );
    assert_eq!(
        entry
            .custom
            .get_binding("PlanQuickActions")
            .expect("legacy custom config is migrated")
            .primary_key,
        Some(KeyCode::ShiftLeft)
    );
}

#[test]
fn load_missing_file_returns_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = KeyConfigStore::load(dir.path().to_str().unwrap()).unwrap();
    assert!(store.configs.is_empty());
}
