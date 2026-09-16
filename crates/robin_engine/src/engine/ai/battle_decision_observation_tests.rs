//! Live battle decision sequencing fixtures.
use super::*;
use crate::ai::{AiEntityHandle, AiState, Decision, Position, Substate};
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{Command, Posture};
use crate::engine::test_support::{
    actors::{make_test_ai_soldier, make_test_pc},
    square_sector,
};
use crate::order::OrderType;

pub(super) fn fixture(disconnected: bool) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.world.fast_grid_mut().size_map(128, 128);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(
            1,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(if disconnected { 175.0 } else { 1000.0 }, 1000.0),
        ),
        0,
    );
    let sector = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let target_sector = if disconnected {
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(
                2,
                0,
                MapPoint::new(200.0, 0.0),
                MapPoint::new(1000.0, 1000.0),
            ),
            0,
        );
        crate::position_interface::SectorHandle::new(2)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap())
    } else {
        sector
    };
    let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(Posture::Upright));
    for (id, x, sector) in [
        (owner, 100.0, sector),
        (
            target,
            if disconnected { 250.0 } else { 650.0 },
            target_sector,
        ),
    ] {
        let entity = engine.get_entity_mut(id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 100.0, 0.0));
        entity.element_data_mut().set_sector(Some(sector));
        entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
    }
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
        "battle_observe.scs",
    ));
    let ai = engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontimeRunning;
    ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
    ai.list_them = vec![target.index()];
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.courage = 0
    });
    ai.hth_weapon_id = 1;
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].distance
        [crate::weapons::WeaponDistance::Default as usize] = 50;
    engine.enter_ai_think_frame(owner);
    (engine, assets, owner, target)
}

fn stop_on_state(engine: &mut EngineInner, assets: &LevelAssets, owner: EntityId) {
    use crate::engine::test_support::asm::*;
    use crate::natives::NativeFn;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .script_class = "ObserveStop".into();
    let quads = vec![
        q_begin_function(0, 3),
        q_native_call(NativeFn::ThisActor as u32),
        q_aff1_native_get_return(0xc000),
        q_aff0_iconstant(0xc004, 0),
        q_aff0_iconstant(0xc008, 1),
        q_native_param(0xc000),
        q_native_param(0xc004),
        q_native_param(0xc008),
        q_native_call(NativeFn::SetCustomNPCValue as u32),
        q_native_param(0xc000),
        q_native_call(NativeFn::StopActor as u32),
        q_return_val(0xc008),
        q_end_function(),
    ];
    let class = crate::scb::ClassEntry {
        source_file: "observe_stop.scs".into(),
        class_name: "ObserveStop".into(),
        size_of_member_variables: 0,
        member_variables: vec![],
        functions: vec![crate::scb::Function {
            name: "FilterAIEvent".into(),
            address: 0,
            num_parameters: 3,
            size_of_return_value: 4,
            size_of_parameters: 12,
            size_of_volatile: 0,
            size_of_temporary: 12,
        }],
        quads,
    };
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![empty_startup_class("observe_stop.scs".into()), class],
        })
        .unwrap(),
    );
    engine.scripts.mission.as_mut().unwrap().bind_actor(
        crate::natives::ScriptHandleCodec::actor_handle(owner),
        "ObserveStop",
    );
    engine.attach_script_bindings(assets);
}

fn pending_moves(
    engine: &EngineInner,
    owner: EntityId,
) -> Vec<(crate::sequence::SequenceId, usize)> {
    engine
        .orders
        .sequence_manager
        .elements_to_go
        .iter()
        .copied()
        .filter(|&(id, index)| {
            engine
                .orders
                .sequence_manager
                .get_element(id, index)
                .is_some_and(|element| {
                    element.owner == Some(owner)
                        && element.state != crate::sequence::SequenceState::Interrupted
                        && matches!(element.command, Command::Move | Command::MoveWaiting)
                })
        })
        .collect()
}

#[test]
fn observe_movement_is_registered_before_the_state_callback() {
    for stops_move in [false, true] {
        let (mut engine, assets, owner, target) = fixture(false);
        let previous_seek = Position {
            x: 300.0,
            y: 350.0,
            ..engine.live_ai_position(owner)
        };
        engine
            .get_entity_mut(owner)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .base
            .seek_position = previous_seek;
        if stops_move {
            stop_on_state(&mut engine, &assets, owner);
        }
        let decision = engine.execute_live_battle_decision(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            Decision::Observe,
            Substate::AttackingReactiontimeRunning,
            0,
            false,
        );
        assert_eq!(decision, Some(Decision::Observe));
        let ai = engine.get_entity(owner).unwrap().enemy_ai().unwrap();
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingApproachToObserve
        );
        assert_eq!(
            ai.base.seek_position, previous_seek,
            "ordinary Observe does not replace the search position"
        );
        assert_eq!(ai.base.when_does_timer_ring, 150);
        assert_eq!(
            ai.base.last_goto_destination,
            engine.live_ai_position(target)
        );
        assert_eq!(ai.base.stop_before_end_of_path_distance, 200);
        let moves = pending_moves(&engine, owner);
        if stops_move {
            assert_eq!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .npc_data()
                    .unwrap()
                    .custom_values[0],
                1
            );
            assert!(
                moves.is_empty(),
                "the callback must stop the already registered Observe route"
            );
        } else {
            assert_eq!(moves.len(), 1);
            let (sequence, index) = moves[0];
            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, index)
                .unwrap();
            assert_eq!(element.owner, Some(owner));
            assert!(matches!(
                element.data,
                crate::sequence::SequenceElementData::Movement {
                    action: OrderType::WalkingUpright,
                    tolerance: 200.0,
                    ..
                }
            ));
        }
        assert_eq!(engine.ai_think_depth(), 1);
    }
}

#[test]
fn failed_fight_executes_observe_on_the_same_think_stack() {
    let (mut engine, assets, owner, target) = fixture(true);
    let sim = crate::sim_rng::test_context();
    assert_eq!(
        engine.select_live_ai_primary_target(
            owner,
            crate::ai_enemy::PrimaryTargetFlags::UNOCCUPIED_PREFERRED
        ),
        Some(AiEntityHandle::new(target.index()))
    );
    let decision = engine.execute_live_battle_decision(
        &sim,
        &assets,
        owner,
        Decision::Fight,
        Substate::AttackingReactiontimeRunning,
        0,
        false,
    );
    assert_eq!(decision, Some(Decision::Observe));
    let ai = engine.get_entity(owner).unwrap().enemy_ai().unwrap();
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachToObserve
    );
    assert!(
        !ai.base.couldnt_reachpoint,
        "failed Fight clears its failure before the already-near Observe call"
    );
    assert!(ai.base.already_on_point);
    assert_eq!(
        ai.base.last_goto_destination,
        engine.live_ai_position(target)
    );
    assert_eq!(ai.base.stop_before_end_of_path_distance, 200);
    assert_eq!(ai.base.when_does_timer_ring, 150);
    assert!(pending_moves(&engine, owner).is_empty());
    assert_eq!(engine.ai_think_depth(), 1);
}
