use super::*;
use crate::ai::{AiLockFlags, PathId, PatrolPath, ReportType};
use crate::ai_enemy::SeekFlags;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

fn group_fixture() -> (EngineInner, LevelAssets, [EntityId; 4]) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(256, 256);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(4000.0, 4000.0)),
        0,
    );
    let sector = crate::ai::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let ids = std::array::from_fn(|index| {
        let mut actor = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        actor.element_data_mut().set_position(WorldPoint3D::new(
            100.0 + index as f32 * 20.0,
            100.0,
            0.0,
        ));
        actor.element_data_mut().set_sector(Some(sector));
        engine.add_test_entity(actor)
    });
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 733;
    let [owner, refused, second, third] = ids;
    let officer = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("test officer"));
    officer.base.current_state = AiState::Seeking;
    officer.base.current_substate = Substate::SeekingOfficerInstructGroupPointing;
    officer.base.seek_position = Position {
        x: 700.0,
        y: 100.0,
        sector: Some(sector),
        level: 0,
    };
    officer.alerted_us = vec![refused.index(), second.index(), third.index()];
    for member in [refused, second, third] {
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(member, format_args!("test member"));
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingGroupGetInstructedByOfficer;
        ai.base.antagonist = Some(AiEntityHandle::new(owner.index()));
    }
    engine
        .world
        .entities
        .expect_ai_controller_mut(refused, format_args!("refusing member"))
        .locks_flag_field = AiLockFlags::FREEZE;
    (engine, assets, ids)
}

#[test]
fn officer_group_instruction_retries_location_first_after_refusal() {
    let (mut engine, assets, [owner, refused, second, third]) = group_fixture();
    engine.execute_ai_officer_instruct_group(&crate::sim_rng::test_context(), &assets, owner);
    let officer = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("test officer"));
    assert_eq!(officer.alerted_us, vec![second.index(), third.index()]);
    assert_eq!(
        officer.base.current_substate,
        Substate::SeekingOfficerWaitForInstructedGroup
    );
    assert_eq!(officer.base.when_does_timer_ring, 763);
    let refused = engine
        .world
        .entities
        .expect_enemy_ai(refused, format_args!("refused member"));
    assert_eq!(
        refused.base.current_substate,
        Substate::SeekingGroupGetInstructedByOfficer
    );
    assert_eq!(refused.base.alert_soldiers_point, Position::default());
    let second = engine
        .world
        .entities
        .expect_enemy_ai(second, format_args!("first accepted member"));
    assert!(
        second.seek_flags.contains(SeekFlags::LOCATION_FIRST),
        "refusal must leave the first-location instruction for the next member"
    );
    assert_eq!(second.base.alert_soldiers_point, officer.base.seek_position);
    let third = engine
        .world
        .entities
        .expect_enemy_ai(third, format_args!("next accepted member"));
    assert!(!third.seek_flags.contains(SeekFlags::LOCATION_FIRST));
    assert_eq!(third.base.alert_soldiers_point, officer.base.seek_position);
}

#[test]
fn officer_group_path_advances_waypoint_on_refusal() {
    let (mut engine, mut assets, [owner, refused, second, third]) = group_fixture();
    let sector = engine.live_ai_position(owner).sector.unwrap();
    assets.navigation.hiking_paths = std::sync::Arc::new(vec![crate::level_data::RawHikingPath {
        waypoints: (0..5)
            .map(|index| crate::level_data::RawWaypoint {
                x: 500 + index * 100,
                y: 200,
                sector: 1,
                level: 0,
                command: crate::level_data::WaypointCommand::None,
            })
            .collect(),
    }]);
    assets.navigation.hiking_waypoint_sectors = Some(std::sync::Arc::new(vec![vec![sector; 5]]));
    let checkpoint = engine
        .world
        .entities
        .expect_ai_controller_mut(refused, format_args!("checkpoint path"));
    checkpoint.has_patrol_path = true;
    checkpoint.patrol_path =
        PatrolPath::new(PathId::new(0).unwrap(), &assets.navigation.hiking_paths);
    let officer = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("checkpoint report"));
    officer.base.my_reconnaissance_report.report_type = ReportType::MissedCharly;
    officer.base.my_reconnaissance_report.charly = Some(AiEntityHandle::new(refused.index()));
    engine.execute_ai_officer_instruct_group(&crate::sim_rng::test_context(), &assets, owner);
    let officer = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("test officer"));
    assert_eq!(officer.alerted_us, vec![second.index(), third.index()]);
    assert_eq!(officer.base.when_does_timer_ring, 763);
    for (member, x) in [(second, 700.0), (third, 900.0)] {
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(member, format_args!("path instruction member"));
        assert_eq!(
            ai.base.alert_soldiers_point.x, x,
            "refused member consumes waypoint zero before the next instruction"
        );
        assert_eq!(ai.base.alert_soldiers_point.y, 200.0);
        assert_eq!(ai.base.alert_soldiers_point.sector, Some(sector));
        assert!(
            ai.seek_flags
                .contains(SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK)
        );
    }
}
