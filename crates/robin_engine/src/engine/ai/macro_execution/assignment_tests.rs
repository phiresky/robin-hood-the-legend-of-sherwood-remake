use super::*;
use crate::position_interface::SectorHandle;

fn fixture() -> (EngineInner, LevelAssets, EntityId) {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        crate::element::Camp::Lacklandists,
    ));
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("path fixture"))
        .script_locked = true;
    (engine, assets, owner)
}

#[test]
fn invalid_patrol_assignment_preserves_original_partial_mutation() {
    use crate::ai::{PathId, PatrolAssignment};

    let (mut engine, assets, owner) = fixture();
    let ai = engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("assignment fixture"));
    ai.has_patrol_path = false;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;

    let assigned = engine.execute_ai_assign_patrol_path(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        PatrolAssignment::Index(PathId::new(3).unwrap()),
        false,
    );
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("assigned owner"));

    assert!(!assigned);
    assert!(
        ai.has_patrol_path,
        "Original sets mbHasPatrolPath before rejecting the index"
    );
    assert!(ai.path_id.is_none());
    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
}

#[test]
fn detached_patrol_status_preserves_original_cursor_history_across_reassignment() {
    use crate::ai::PatrolAssignment;
    use crate::ai::{PathHistoryEntry, PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    // The original game writes current, last, direction, path
    // and history in the original game, while initialization resets only
    // current/direction. Patrol-path assignment uses that initialization path in
    // the original-game AI state.
    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 10,
                y: 20,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 30,
                y: 40,
                sector: 2,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];
    let path_id = PathId::new(0).unwrap();
    let history = vec![PathHistoryEntry {
        position: Position {
            x: 7.0,
            y: 9.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: 3,
        distance: 11,
    }];
    let mut path = PatrolPath::new(path_id, &paths).unwrap();
    path.current_waypoint_index = 1;
    path.last_waypoint_index = 9;
    path.forward = false;
    path.history = history.clone();

    let (mut engine, assets, owner) = fixture();
    let ai = engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("assignment fixture"));
    ai.has_patrol_path = true;
    ai.patrol_path = Some(path);
    ai.detach_patrol_path(None, false);
    assert!(ai.patrol_path.is_none());
    assert_eq!(ai.detached_patrol_path_status.current_waypoint_index, 1);
    assert_eq!(ai.detached_patrol_path_status.last_waypoint_index, 9);
    assert!(!ai.detached_patrol_path_status.forward);
    assert_eq!(ai.detached_patrol_path_status.history.len(), 1);
    assert_eq!(ai.detached_patrol_path_status.history[0].direction, 3);
    assert_eq!(ai.detached_patrol_path_status.history[0].distance, 11);

    let mut assets = assets;
    assets.navigation.hiking_paths = std::sync::Arc::new(paths);
    assert!(engine.execute_ai_assign_patrol_path(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        PatrolAssignment::Index(path_id),
        false
    ));
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("assigned owner"));
    let restored = ai.patrol_path.as_ref().unwrap();
    assert_eq!(restored.current_waypoint_index, 0);
    assert_eq!(restored.last_waypoint_index, 9);
    assert!(restored.forward);
    assert_eq!(restored.history.len(), 1);
    assert_eq!(restored.history[0].direction, 3);
    assert_eq!(restored.history[0].distance, 11);
}

#[test]
fn script_way_assignment_keeps_special_action() {
    // Script-level path assignment routes through the original game's
    // hiking patrol-path assignment overload. Its valid-path branch
    // clears only the likes-to-sit flag; special-action state survives, so a
    // leisure-authored NPC on a scripted route later fails movement's
    // already-on-point shortcut and runs a real zero-length move when
    // returning to its route (contrasting with the
    // index-based path-assignment variant).
    use crate::ai::{PathId, PatrolAssignment};

    let paths = vec![crate::level_data::RawHikingPath {
        waypoints: vec![crate::level_data::RawWaypoint {
            x: 10,
            y: 20,
            sector: 1,
            level: 0,
            command: crate::level_data::WaypointCommand::None,
        }],
    }];

    let (mut engine, assets, owner) = fixture();
    let ai = engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("assignment fixture"));
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let mut assets = assets;
    assets.navigation.hiking_paths = std::sync::Arc::new(paths.clone());
    let assigned = engine.execute_ai_assign_patrol_path(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        PatrolAssignment::ScriptWay(PathId::new(0).unwrap()),
        true,
    );
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("assigned owner"));
    assert!(assigned);
    assert!(ai.has_patrol_path);
    assert!(
        ai.special_action,
        "the hiking-path variant must not clear the special-action flag"
    );
    assert!(!ai.likes_to_sit_around);

    // The waypoint-macro index overload clears both flags.
    let (mut engine, assets, owner) = fixture();
    let ai = engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("assignment fixture"));
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let mut assets = assets;
    assets.navigation.hiking_paths = std::sync::Arc::new(paths.clone());
    let assigned = engine.execute_ai_assign_patrol_path(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        PatrolAssignment::Index(PathId::new(0).unwrap()),
        false,
    );
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("assigned owner"));
    assert!(assigned);
    assert!(
        !ai.special_action,
        "the 16-bit index variant clears the special-action flag"
    );
    assert!(!ai.likes_to_sit_around);
}
