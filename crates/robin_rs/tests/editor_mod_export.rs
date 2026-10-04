//! The fixture is produced by the editor's browser bake acceptance test.
//! It exercises the same archive discovery and terrain decoders as installed mods.

use std::sync::Arc;

use robin_engine::level_data::LoadedLevel;
use robin_engine::sbfile::SbFileSystem;
use robin_rs::level_loading_host::{
    pre_decode_background_map_with_files, pre_decode_minimap_with_files,
};
use robin_rs::mod_pack::{enumerate_missions, mount_mod_overlay, scan_mods_dir};
use robin_util::asset_fs::AssetVfs;

#[test]
#[ignore = "requires ROBIN_EDITOR_MAP_ZIP from a full editor bake"]
fn full_editor_archive_constructs_native_map_without_base_datadir() {
    use robin_engine::engine::{Engine, EngineArgs, LevelAssets, LevelLoadArgs, SimConfig};
    let archive = std::path::PathBuf::from(std::env::var("ROBIN_EDITOR_MAP_ZIP").unwrap());
    let directory = tempfile::tempdir().unwrap();
    let installed = directory.path().join("compiled-map.zip");
    std::fs::copy(&archive, &installed).unwrap();
    let mods = scan_mods_dir(directory.path());
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].details.hackable_missions.len(), 1);
    let name = &mods[0].details.hackable_missions[0];
    let files = SbFileSystem::new(Arc::new(AssetVfs::new()));
    mount_mod_overlay(&files, &installed).unwrap();
    assert_eq!(enumerate_missions(&mods, &files).len(), 1);
    let bytes = files
        .read_shared(&format!("Data/Levels/{name}.level.json"))
        .unwrap();
    let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(descriptor["spawn_points"], serde_json::json!([]));
    assert!(descriptor.get("spawn_player").is_none());
    let editor = files
        .read_shared(&format!("editor/{name}.rhlos-map.json"))
        .unwrap();
    let scene: serde_json::Value = serde_json::from_slice(&editor).unwrap();
    assert_eq!(scene["map"], descriptor["title"]);
    assert!(
        scene["assetSources"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    );
    let loaded = LoadedLevel::hackable_from_json(&bytes).unwrap();
    assert_eq!(&loaded.mission.header.map_filename, name);
    assert!(loaded.mission.beam_mes.is_empty());
    assert!(loaded.mission.soldiers.is_empty());
    assert!(loaded.mission.civilians.is_empty());
    assert!(loaded.mission.bonuses.is_empty());
    assert!(loaded.mission.pcs_to_rescue.is_empty());
    assert!(loaded.mission.scrolls.is_empty());
    assert!(loaded.mission.script_objects.is_none());
    assert!(loaded.mission.hiking_paths.is_empty());
    let background =
        pre_decode_background_map_with_files(name, "Day", "Data/Levels", None, &mut |_| {}, &files)
            .unwrap()
            .unwrap();
    let dimensions = (background.width, background.height);
    let pixel_count = usize::from(dimensions.0) * usize::from(dimensions.1);
    assert_eq!(background.pixels.len(), pixel_count);
    assert_eq!(
        background.occlusion_depth.as_ref().unwrap().len(),
        pixel_count
    );
    assert!(
        background.pixels.windows(2).any(|p| p[0] != p[1]),
        "map image is constant"
    );
    let minimap =
        pre_decode_minimap_with_files(name, "Day", "Data/Levels", None, &mut |_| {}, &files)
            .unwrap();
    assert!(minimap.width > 0 && minimap.height > 0);
    let mut assets = LevelAssets::new();
    let mut profiles = robin_engine::profiles::ProfileManager::new();
    let mut campaign = robin_engine::campaign::Campaign::new();
    let index = campaign
        .force_next_mission_by_name(&mut profiles, name, name, true)
        .unwrap();
    campaign.current_mission_idx = Some(index);
    assets.profile_manager = Arc::new(profiles);
    let engine = Engine::new(EngineArgs {
        campaign,
        level: LevelLoadArgs {
            assets: &mut assets,
            level_directory: "",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: (f32::from(dimensions.0), f32::from(dimensions.1)),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: vec![],
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: SimConfig {
            script_enabled: false,
            ..Default::default()
        },
    })
    .expect("construct ZIP map without a base datadir");
    let grid = engine.fast_grid();
    assert!(!grid.level.blocks.is_empty());
    assert!(!grid.level.sectors.is_empty());
    for region in &background.appearance_regions {
        assert!(
            region
                .patches
                .iter()
                .all(|&patch| usize::from(patch) < engine.patches().len()),
            "appearance region references a missing native control"
        );
        assert_eq!(region.states.len(), 1 << region.patches.len());
        let pixels = usize::from(region.bounds[2]) * usize::from(region.bounds[3]);
        for state in &region.states {
            assert_eq!(state.color.len(), pixels);
            assert_eq!(state.depth.len(), pixels);
        }
    }
    eprintln!(
        "{name}: {}x{}, {} sight obstacles, {} masks, {} door projections, {} grid blocks, {} appearance regions for {} controls; color/depth/minimap and editor scene loaded without a base datadir",
        dimensions.0,
        dimensions.1,
        assets.environment.static_sight_obstacles.len(),
        grid.level.masks.len(),
        grid.level.door_projection_infos.len(),
        grid.level.blocks.len(),
        background.appearance_regions.len(),
        engine.patches().len()
    );
}

#[test]
fn browser_compiled_map_loads_geometry_without_mission_spawns() {
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("editor-bake-contract.zip");
    std::fs::write(
        &archive,
        include_bytes!("fixtures/editor-bake-contract.zip"),
    )
    .unwrap();
    let mods = scan_mods_dir(directory.path());
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].details.hackable_missions, ["editor-bake-contract"]);
    let files = SbFileSystem::new(Arc::new(AssetVfs::new()));
    mount_mod_overlay(&files, &archive).unwrap();
    let missions = enumerate_missions(&mods, &files);
    assert_eq!(missions.len(), 1);
    assert!(missions[0].hackable);

    let bytes = files
        .read_shared("Data/Levels/editor-bake-contract.level.json")
        .unwrap();
    let level = LoadedLevel::hackable_from_json(&bytes).unwrap();
    assert_eq!(level.mission.header.map_filename, "editor-bake-contract");
    assert!(level.mission.beam_mes.is_empty());
    assert_eq!(level.proto.sight_obstacles.len(), 1);
    let point = &level.proto.sight_obstacles[0].points[0];
    assert_eq!((point.x, point.y, point.z_top), (20.0, 40.0, 20.0));
    let motion = level.proto.motion_data.as_ref().unwrap();
    assert_eq!(motion.layers[0][0].obstacles.len(), 1);
    assert!(motion.layers[1].is_empty());

    let background = pre_decode_background_map_with_files(
        "editor-bake-contract",
        "Day",
        "Data/Levels",
        None,
        &mut |_| {},
        &files,
    )
    .unwrap()
    .unwrap();
    assert_eq!((background.width, background.height), (1100, 128));
    assert_eq!(background.pixels[30 * 1100 + 30], 0xf800);
    assert_eq!(background.pixels[30 * 1100 + 1023], 0x07e0);
    assert_eq!(background.pixels[30 * 1100 + 1024], 0x07e0);
    let depth = background.occlusion_depth.unwrap();
    let expected = ((50.5_f32 / 128.0) * 65535.0).round() as u16;
    assert!(depth[30 * 1100 + 30].abs_diff(expected) <= 2);
    let minimap = pre_decode_minimap_with_files(
        "editor-bake-contract",
        "Day",
        "Data/Levels",
        None,
        &mut |_| {},
        &files,
    )
    .unwrap();
    assert_eq!((minimap.width, minimap.height), (79, 9));
}
