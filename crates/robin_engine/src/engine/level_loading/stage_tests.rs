use super::*;

#[test]
fn graph_free_mission_installs_actor_footprints_before_spawning() {
    let mut loaded = crate::level_data::LoadedLevel::hackable_from_json(include_bytes!(
        "../../../../../mods/multi-team-demos/Data/Levels/MultiTeamFourGrades.level.json"
    ))
    .expect("four-grades descriptor");
    assert_eq!(loaded.mission.soldiers.len(), 48);
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut staging = LevelLoadStaging::default();

    engine
        .load_motion_stage(&mut assets, &mut staging, &mut loaded, (2508.0, 2508.0))
        .expect("graph-free motion stage");

    let footprint = crate::coordinates::MoveBoxHalfDiagonal::new(6.0, 4.0);
    assert_eq!(
        engine.world.fast_grid.try_move_box_half_diagonal(0),
        Some(footprint)
    );
    let graph = &assets.navigation.pathfinder_graph;
    assert_eq!(graph.static_data.half_diagonals, vec![footprint]);
    assert!(
        graph.nodes.is_empty(),
        "routing still uses the visibility graph"
    );
    assert_eq!(
        graph.find_area_at_point(0, MapPoint::new(1120.0, 1540.0)),
        Some(0)
    );
    assert_eq!(engine.world.fast_grid.try_move_box_half_diagonal(1), None);
}

#[test]
fn initializer_reports_each_ordered_stage_boundary_once() {
    let config = SimConfig {
        script_enabled: false,
        ..Default::default()
    };
    let sim = crate::sim_rng::SimulationContext::with_seed_and_config(7, config);
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut staging = LevelLoadStaging::default();
    let mut progress_updates = Vec::new();

    engine
        .initialize_from_mission(
            &sim,
            &mut assets,
            &mut staging,
            "stage-order-test",
            "stage-order-proto",
            crate::level_data::LoadedLevel::empty(),
            "Data/Levels",
            (0.0, 0.0),
            &mut |progress| progress_updates.push(progress),
        )
        .expect("empty no-script mission should traverse every load stage");

    assert_eq!(progress_updates, vec![1.0; 8]);
}
