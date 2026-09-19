//! Save store tests. Shared fixtures live here; the store/recovery and
//! capture/replay-boundary suites are split into child modules.
use super::*;
use crate::game::Game;
use crate::host::ApplicationContext;
use crate::key_config_store::KeyConfigStore;
use crate::save_file::special_slots;
use robin_engine::campaign::Campaign;
use robin_engine::mission::Mission;
use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};
use robin_engine::profiles::{MissionProfile, ProfileManager};

mod capture;
mod store;

fn published_slot(filename: &str) -> SaveGame {
    let mut slot = SaveGame::new(filename.into(), filename.into(), 1);
    slot.timestamp = "123".into();
    slot.mission_name = "Mission 1".into();
    slot.player_profile_id = Some(0);
    slot.player_name = "Player".into();
    slot.campaign_progress = Some(0);
    slot.missions_done = Some(0);
    slot.missions_total = Some(1);
    slot.gang_size = Some(1);
    slot.ransom = Some(0);
    slot.blazons = Some(0);
    slot.amulets = Some(0);
    slot.validate_published_metadata().unwrap();
    slot
}

fn indexed_store(root: &Path, names: &[&str]) -> SaveGameManager {
    let mut manager = SaveGameManager::new(root.to_str().unwrap().into());
    for name in names {
        manager.insert_test_slot(published_slot(name), SlotState::Published);
    }
    manager.save_index().unwrap();
    manager
}

use robin_engine::test_support::fresh_engine;

fn fresh_save_session(
    player_name: &str,
) -> (Engine, engine_api::LevelAssets, ProfileManager, Host) {
    let mut profiles = ProfileManager::default();
    let mut campaign = Campaign::default();
    for mission_id in [1, 3, 17] {
        let profile_idx = profiles.missions.len() as u32;
        profiles.missions.push(MissionProfile {
            id: mission_id,
            mission_filename: format!("Mission_{mission_id}"),
            proto_level_filename: format!("Map_{mission_id}"),
            mission_name: format!("Mission {mission_id}"),
            ..MissionProfile::default()
        });
        campaign.missions.push(Mission {
            profile_idx: Some(profile_idx),
            ..Mission::default()
        });
    }
    let mut assets = engine_api::LevelAssets::new();
    let engine = Engine::new_for_test(800.0, 600.0, campaign, &mut assets).expect("engine");

    let save_root = format!("/tmp/save-metadata-{player_name}");
    let mut players = PlayerProfileManager::new(save_root.clone());
    let player = players.create_profile(player_name.to_string(), DifficultyLevel::Medium);
    players.set_active(player);
    let application_context = ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(&save_root),
        engine_api::GlobalOptions::default(),
        players,
        KeyConfigStore::new(save_root),
        None,
    )
    .expect("complete test application context");
    let host = Host::new(application_context.try_into().unwrap(), 800.0, 600.0).unwrap();
    (engine, assets, profiles, host)
}

#[test]
fn mission_clock_metadata_excludes_sherwood_and_survives_catalog_roundtrip() {
    let (engine, _, mut profiles, _) = fresh_save_session("clock-metadata");
    let mut campaign = engine.campaign().clone();
    campaign.set_value(CampaignValue::MissionLength, 3903);
    profiles.missions[0].location = robin_engine::profiles::MissionLocation::Nottingham;
    let mut slot = SaveGame::new("Savegame_000".into(), "Mission".into(), 1);
    slot.update_campaign_metadata(&campaign, &profiles);
    assert_eq!(slot.mission_elapsed_seconds, Some(3903));
    let encoded = serde_json::to_value(&slot).unwrap();
    assert_eq!(
        serde_json::from_value::<SaveGame>(encoded.clone()).unwrap(),
        slot
    );
    let mut legacy = encoded;
    legacy
        .as_object_mut()
        .unwrap()
        .remove("mission_elapsed_seconds");
    assert_eq!(
        serde_json::from_value::<SaveGame>(legacy)
            .unwrap()
            .mission_elapsed_seconds,
        None
    );
    campaign.set_value(CampaignValue::MissionLength, 0);
    slot.update_campaign_metadata(&campaign, &profiles);
    assert_eq!(slot.mission_elapsed_seconds, Some(0));
    profiles.missions[0].location = robin_engine::profiles::MissionLocation::Sherwood;
    slot.update_campaign_metadata(&campaign, &profiles);
    assert_eq!(
        slot.mission_elapsed_seconds, None,
        "ordinary Sherwood saves omit the clock too"
    );
}

fn game_for_save(profiles: &ProfileManager, mission_id: u32) -> Game {
    let profile = profiles
        .missions
        .iter()
        .find(|profile| profile.id == mission_id)
        .expect("test mission profile");
    let mut game = Game::default();
    game.set_mission_assets(
        robin_engine::mission_assets::MissionAssetDescriptor::built_in(
            profile.mission_filename.clone(),
            profile.proto_level_filename.clone(),
            profile.proto_level_filename.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    game
}

#[test]
fn create_and_find() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    let idx = mgr.create("My Save".into(), 42);
    assert_eq!(idx, 0);
    assert_eq!(mgr.count(), 1);
    assert_eq!(mgr.get(0).unwrap().text, "My Save");
    assert_eq!(mgr.get(0).unwrap().mission_id, 42);
    assert_eq!(mgr.get(0).unwrap().filename, "Savegame_000");
    assert_eq!(mgr.find_by_name("My Save"), Some(0));
    assert_eq!(mgr.find_by_name("Nope"), None);
}

#[test]
fn display_metadata_copy_rejects_missing_slots() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    let slot = mgr.create("My Save".into(), 42);
    let before = mgr.saves().cloned().collect::<Vec<_>>();

    let missing_source = mgr.copy_display_metadata(usize::MAX, slot).unwrap_err();
    // Stable-identity lookup rejects the missing source before attempting
    // to read its explicit lifecycle state or copy any metadata.
    assert_eq!(
        missing_source.to_string(),
        format!("missing save slot {}", usize::MAX)
    );

    let missing_destination = mgr.copy_display_metadata(slot, usize::MAX).unwrap_err();
    assert!(
        missing_destination
            .to_string()
            .contains("cannot copy metadata to missing save slot")
    );
    assert!(mgr.saves().eq(before.iter()));
}

#[test]
fn snapshot_copy_preserves_only_destination_identity() {
    let mut source = published_slot("QuickSave");
    source.multiplayer_diagnostic = true;
    source.player_name = "Source player".into();
    for name in ["Continue", "ExQuickSave", "Manual"] {
        let destination = SaveGame::new(name.into(), "Keep my label".into(), 99);
        let copied = source.cloned_for_slot(&destination);
        let mut expected = serde_json::to_value(&source).unwrap();
        let destination_json = serde_json::to_value(&destination).unwrap();
        for field in ["filename", "text", "special"] {
            expected[field] = destination_json[field].clone();
        }
        assert_eq!(serde_json::to_value(copied).unwrap(), expected);
        assert_eq!(source.filename, "QuickSave");
        assert_eq!(destination.mission_id, 99);
    }
}

#[test]
fn special_slots() {
    let save = SaveGame::new("Continue".into(), "Continue".into(), 0);
    assert!(save.is_special());
    assert!(save.is_continue());
    assert!(!save.is_restart());
}

#[test]
fn special_auto_detect() {
    let save = SaveGame::new("Restart".into(), "Restart Save".into(), 0);
    assert!(save.is_special());
    assert!(save.is_restart());
    assert!(!save.is_continue());
}

#[test]
fn non_special_filename() {
    let save = SaveGame::new("Savegame_005".into(), "My Save".into(), 0);
    assert!(!save.is_special());
    assert_eq!(save.version, save_file::SAVE_FORMAT_VERSION);
}

#[test]
fn autosave_storage_names_are_strict_and_path_safe() {
    for valid in ["Autosave_1_0000", "Autosave_18446744073709551615_9999"] {
        assert!(is_generated_autosave_filename(valid), "{valid}");
        assert_eq!(
            SpecialSlot::from_filename(valid),
            Some(SpecialSlot::Autosave)
        );
    }
    for invalid in [
        "Autosave_1_999",
        "Autosave_1_0000.json",
        "Autosave_../0000",
        "Autosave_1_../../Continue",
        "Autosave_notes",
        "autosave_1_0000",
    ] {
        assert!(!is_generated_autosave_filename(invalid), "{invalid}");
        assert_eq!(SpecialSlot::from_filename(invalid), None, "{invalid}");
    }
}

#[test]
fn find_or_create() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    let idx1 = mgr.find_or_create_by_filename("Continue", "Continue 1");
    assert_eq!(idx1, 0);
    assert_eq!(mgr.count(), 1);
    assert!(mgr.get(0).unwrap().is_continue());

    // Same filename → updates text, same index
    let idx2 = mgr.find_or_create_by_filename("Continue", "Continue 2");
    assert_eq!(idx2, 0);
    assert_eq!(mgr.count(), 1);
    assert_eq!(mgr.get(0).unwrap().text, "Continue 2");
}

#[test]
fn serde_round_trip() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    mgr.create("Save 1".into(), 10);
    mgr.create("Save 2".into(), 20);

    let json = serde_json::to_string(&SaveIndex {
        saves: mgr.catalog.iter().cloned().collect(),
        next_id: mgr.next_id,
        save_directory: mgr.save_directory.clone(),
    })
    .unwrap();
    let mgr2: SaveIndex = serde_json::from_str(&json).unwrap();
    assert_eq!(mgr2.saves.len(), 2);
    assert_eq!(mgr2.saves[0].text, "Save 1");
    assert_eq!(mgr2.saves[1].mission_id, 20);
}

#[test]
fn auto_incrementing_filenames() {
    let mut mgr = SaveGameManager::new("/tmp/test_saves".into());
    mgr.create("A".into(), 1);
    mgr.create("B".into(), 2);
    mgr.create("C".into(), 3);
    assert_eq!(mgr.catalog[0].filename, "Savegame_000");
    assert_eq!(mgr.catalog[1].filename, "Savegame_001");
    assert_eq!(mgr.catalog[2].filename, "Savegame_002");
}

#[test]
fn full_and_thumb_paths() {
    let mut mgr = SaveGameManager::new("/saves/profile_1".into());
    mgr.create_with_filename("Continue".into(), "Continue".into(), 5);
    assert_eq!(
        mgr.save_path(0),
        PathBuf::from("/saves/profile_1/Continue.json")
    );
    assert_eq!(
        mgr.thumb_path(0),
        PathBuf::from("/saves/profile_1/Continue_thumb.png")
    );
}
