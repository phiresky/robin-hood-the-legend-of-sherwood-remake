use super::*;
use crate::ai::{PathId, PatrolPath, ReportType};
use crate::ai_enemy::SeekFlags;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

#[test]
fn alert_camp_roster_preserves_load_order_and_reads_live_membership() {
    use crate::element::Camp;

    let (mut engine, assets, [owner, first, second, third]) = group_fixture();
    let foreign = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    engine.ai.global.all_soldier_handles = std::sync::Arc::new(vec![
        third.index(),
        foreign.index(),
        owner.index(),
        first.index(),
        second.index(),
    ]);
    let sim = crate::sim_rng::test_context();
    let execution = AlertExecution {
        engine: &mut engine,
        sim: &sim,
        assets: &assets,
        owner,
    };
    let count = execution.camp_members(Camp::Lacklandists).count();
    assert_eq!(count, 4);
    assert_eq!(
        execution.camp_members(Camp::Lacklandists).next(),
        Some(third)
    );
    // Removing another camp cannot consume a position in this camp's traversal.
    execution.engine.world.entities.remove(foreign);
    assert_eq!(
        execution.camp_members(Camp::Lacklandists).nth(1),
        Some(owner)
    );
    // A recipient callback may remove a member already visited; later indices
    // address the compacted live camp roster, while the loop keeps its count.
    execution.engine.world.entities.remove(third);
    assert_eq!(
        execution.camp_members(Camp::Lacklandists).nth(1),
        Some(first)
    );
    assert_eq!(
        execution.camp_members(Camp::Lacklandists).nth(2),
        Some(second)
    );
    assert_eq!(
        execution.camp_members(Camp::Lacklandists).nth(count - 1),
        None
    );
}

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
        .current_substate = Substate::SeekingJustWatching;
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
    assert_eq!(refused.base.current_substate, Substate::SeekingJustWatching);
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

#[test]
fn officer_group_path_reassignment_uses_live_waypoints_with_initial_stride() {
    use crate::engine::test_support::asm::*;
    use crate::engine::types::MissionScript;
    use crate::natives::{NativeFn, ScriptHandleCodec};
    use crate::scb::{ClassEntry, Function, ScbFile};
    let (mut engine, mut assets, [owner, refused, second, third]) = group_fixture();
    let sector = engine.live_ai_position(owner).sector.unwrap();
    assets.navigation.hiking_paths = std::sync::Arc::new(
        [(5, 500, 100), (7, 1000, 200)]
            .into_iter()
            .map(|(count, start, step)| crate::level_data::RawHikingPath {
                waypoints: (0..count)
                    .map(|index| crate::level_data::RawWaypoint {
                        x: start + index * step,
                        y: 200,
                        sector: 1,
                        level: 0,
                        command: crate::level_data::WaypointCommand::None,
                    })
                    .collect(),
            })
            .collect(),
    );
    assets.navigation.hiking_waypoint_sectors =
        Some(std::sync::Arc::new(vec![vec![sector; 5], vec![sector; 7]]));
    let checkpoint = engine
        .world
        .entities
        .expect_ai_controller_mut(refused, format_args!("checkpoint"));
    checkpoint.has_patrol_path = true;
    checkpoint.patrol_path =
        PatrolPath::new(PathId::new(0).unwrap(), &assets.navigation.hiking_paths);
    let officer = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("checkpoint report"));
    officer.base.my_reconnaissance_report.report_type = ReportType::MissedCharly;
    officer.base.my_reconnaissance_report.charly = Some(AiEntityHandle::new(refused.index()));
    let handle = ScriptHandleCodec::actor_handle(refused);
    engine
        .world
        .entities
        .expect_entity_mut(refused, format_args!("scripted checkpoint"))
        .actor_data_mut()
        .unwrap()
        .script_class = "ChangePath".into();
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![
                ClassEntry {
                    source_file: "path.scs".into(),
                    class_name: "StartUp".into(),
                    size_of_member_variables: 0,
                    member_variables: vec![],
                    functions: vec![],
                    quads: vec![],
                },
                ClassEntry {
                    source_file: "path.scs".into(),
                    class_name: "ChangePath".into(),
                    size_of_member_variables: 0,
                    member_variables: vec![],
                    functions: vec![Function {
                        name: "FilterAIEvent".into(),
                        address: 0,
                        num_parameters: 3,
                        size_of_return_value: 4,
                        size_of_parameters: 12,
                        size_of_volatile: 0,
                        size_of_temporary: 8,
                    }],
                    quads: vec![
                        q_begin_function(0, 2),
                        q_aff0_iconstant(0xC000, handle),
                        q_aff0_iconstant(0xC004, 1),
                        q_native_param(0xC000),
                        q_native_param(0xC004),
                        q_native_call(NativeFn::AssignPath as u32),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_end_function(),
                    ],
                },
            ],
        })
        .expect("path reassignment script compiles"),
    );
    engine.attach_script_bindings(&assets);
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .bind_actor(handle, "ChangePath");
    engine.execute_ai_officer_instruct_group(&crate::sim_rng::test_context(), &assets, owner);
    assert_eq!(
        engine
            .world
            .entities
            .expect_ai_controller(refused, format_args!("reassigned checkpoint"))
            .patrol_path
            .as_ref()
            .unwrap()
            .hiking_path_index,
        PathId::new(1).unwrap()
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("officer"))
            .alerted_us,
        vec![second.index(), third.index()]
    );
    for (member, expected_x) in [(second, 1400.0), (third, 1800.0)] {
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(member, format_args!("path recipient"));
        assert_eq!(
            ai.base.alert_soldiers_point.x, expected_x,
            "later instructions read reassigned path at the stride captured before callbacks"
        );
        assert_eq!(ai.base.alert_soldiers_point.sector, Some(sector));
    }
}
