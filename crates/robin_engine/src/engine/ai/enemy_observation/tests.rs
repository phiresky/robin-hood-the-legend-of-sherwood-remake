use super::*;
use crate::ai::{AiEntityHandle, AiState, AlertLevel, Position, Substate};
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{
    ActionState, Camp, ElementBonus, ElementData, ElementKind, Entity, ObjectData, Posture,
};
use crate::element_kinds::ObjectType;
use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_pc};
use crate::order::OrderType;

fn add_ale(engine: &mut EngineInner, owner: EntityId, x: f32, y: f32) -> EntityId {
    let mut element = ElementData::default();
    element.kind = ElementKind::ObjectOther;
    element.active = true;
    element.set_position(WorldPoint3D::new(x, y, 0.0));
    let sector = engine.live_ai_position(owner).sector;
    element.set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
    engine.add_test_entity(Entity::Bonus(ElementBonus {
        element,
        object: ObjectData {
            object_type: ObjectType::Ale,
            ..Default::default()
        },
    }))
}

#[test]
fn ale_competition_uses_first_qualifying_npc_registration_and_one_los_query() {
    let (mut engine, mut assets, owner, _) = fixture();
    let mut civilian = crate::engine::test_support::actors::make_test_civilian(Posture::Upright);
    civilian.npc_data_mut().unwrap().ai_brain = crate::element::AiBrain::Friendly(Box::default());
    let first = engine.add_test_entity(civilian);
    let second = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let bottle = add_ale(&mut engine, owner, 400.0, 100.0);
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let sector = engine.live_ai_position(owner).sector;
    for (id, x) in [(first, 200.0), (second, 300.0)] {
        let entity = engine.ent_mut(id);
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 100.0, 0.0));
        let ai = entity.ai_controller_mut().unwrap();
        ai.current_substate = Substate::WonderingAleReactiontime;
        ai.interesting_object = Some(AiEntityHandle::new(bottle.index()));
    }
    let entity = engine.ent_mut(owner);
    entity.element_data_mut().active = true;
    entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(1.0, 0.0),
    );
    let npc = entity.ai_actor_data_mut().unwrap();
    npc.view_radius = 500;
    npc.view_direction = [1.0, 0.0];
    npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    entity.ai_controller_mut().unwrap().interesting_object =
        Some(AiEntityHandle::new(bottle.index()));
    crate::sight_obstacle::begin_parity_visibility_capture();
    let unavailable = engine.unavailable_ale_position(&assets, owner);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(
        unavailable,
        Some(engine.live_ai_position(first)),
        "the earlier civilian wins before the closer soldier in another typed arena"
    );
    assert_eq!(
        queries.len(),
        1,
        "the first qualifying registration ends the scan"
    );
    assert_eq!(queries[0].destination[0], 200.0);
}

#[test]
fn inactive_ale_approach_faces_current_bottle_position() {
    let (mut engine, assets, owner, _) = fixture();
    let bottle = add_ale(&mut engine, owner, 400.0, 300.0);
    engine.set_active(bottle, false);
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingAle;
    ai.base.interesting_object = Some(AiEntityHandle::new(bottle.index()));
    ai.base.seek_position = Position {
        x: 100.0,
        y: 500.0,
        ..Position::default()
    };
    assert_eq!(
        engine.unavailable_ale_position(&assets, owner),
        Some(engine.observation_object_position(bottle))
    );
    engine.execute_ai_ale_approach(&crate::sim_rng::test_context(), &assets, owner, true);
    let expected_direction =
        crate::position_interface::vector_to_sector_0_to_15_iso(300.0, 200.0) as u32;
    assert!(engine.orders.sequence_manager.sequences_iter().flat_map(|s| s.elements.iter())
        .any(|e| e.owner == Some(owner) && matches!(e.get_property(crate::sequence::Field::Direction),
            Some(crate::sequence::FieldValue::Integer(direction)) if *direction == expected_direction)));
    let ai = engine.observation_ai(owner);
    assert_eq!(ai.base.current_substate, Substate::WonderingAleAway);
    assert_eq!(ai.base.when_does_timer_ring, 130);
    assert_eq!(
        ai.base.current_emoticon_type,
        crate::ai::EmoticonType::Thunderstorm
    );
}

#[test]
fn enemy_below_reads_ground_position_at_the_call_boundary() {
    let (mut engine, _, owner, target) = fixture();
    for (me, other, below) in [
        (
            WorldPoint3D::new(572.0, 2465.001, 105.00101),
            WorldPoint3D::new(577.0, 2472.219, 104.21895),
            false,
        ),
        (
            WorldPoint3D::new(572.0, 2465.001, 105.00101),
            WorldPoint3D::new(572.0, 2465.001, 104.0),
            true,
        ),
        (
            WorldPoint3D::new(1061.2039, 2015.2788, 150.001),
            WorldPoint3D::new(1031.4138, 2002.76, 107.499275),
            true,
        ),
        (
            WorldPoint3D::new(1061.2039, 2020.2788, 150.001),
            WorldPoint3D::new(1031.4138, 2002.76, 107.499275),
            false,
        ),
    ] {
        place(&mut engine, owner, me);
        place(&mut engine, target, other);
        assert_eq!(engine.observation_enemy_below(owner, target), below);
    }
}

#[test]
fn arrow_reaction_keeps_rank_and_existing_search_branches_distinct() {
    for (state, rank, expected, delay) in [
        (
            AiState::Seeking,
            ProfileRank::Soldier,
            Some(Substate::SeekingArrowReactiontime),
            1,
        ),
        (
            AiState::Default,
            ProfileRank::Officer,
            Some(Substate::SeekingArrowJustWatching),
            crate::parameters_ai::AI_FIRST_LOOK_TIME as u32,
        ),
        (AiState::Default, ProfileRank::None, None, 0),
    ] {
        let (mut engine, mut assets, owner, _) = fixture();
        let origin = Position {
            x: 600.0,
            y: 600.0,
            ..engine.live_ai_position(owner)
        };
        let ai = engine.observation_ai_mut(owner);
        ai.base.current_state = state;
        ai.base.current_substate = if state == AiState::Seeking {
            Substate::SeekingSeekpoint
        } else {
            Substate::DefaultOnPost
        };
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.rank = rank
        });
        ai.base.timer_is_running = false;
        let entry_substate = ai.base.current_substate;
        engine.execute_ai_received_arrow(&crate::sim_rng::test_context(), &assets, owner, origin);
        let ai = engine.observation_ai(owner);
        assert_eq!(ai.base.my_reconnaissance_report.seek_position, origin);
        assert_eq!(ai.base.current_substate, expected.unwrap_or(entry_substate));
        if expected.is_some() {
            assert_eq!(ai.base.when_does_timer_ring, 100 + delay);
        } else {
            assert!(!ai.base.timer_is_running);
        }
    }
}

#[test]
fn ale_timer_reads_retained_inactive_bottle_instead_of_cached_seek_position() {
    let (mut engine, mut assets, owner, _) = fixture();
    let mut element = ElementData::default();
    element.kind = ElementKind::ObjectOther;
    element.active = false;
    element.set_sector_topology(
        engine.live_ai_position(owner).sector,
        crate::fast_find_grid::SectorIndex::new(0),
    );
    element.set_position(WorldPoint3D::new(632.4453, 1835.14, 0.0));
    let bottle = engine.add_test_entity(Entity::Bonus(ElementBonus {
        element,
        object: ObjectData {
            object_type: ObjectType::Ale,
            ..Default::default()
        },
    }));
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingAleReactiontime;
    ai.base.interesting_object = Some(AiEntityHandle::new(bottle.index()));
    ai.base.seek_position = Position {
        x: 200.0,
        y: 200.0,
        ..ai.base.seek_position
    };
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.beer = 1
    });
    engine.execute_ai_ale_reaction(&crate::sim_rng::test_context(), &assets, owner);
    let ai = engine.observation_ai(owner);
    assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
    assert_eq!(
        ai.base.object_of_desire,
        Some(AiEntityHandle::new(bottle.index()))
    );
    assert_eq!(
        ai.base.last_goto_destination.map_point(),
        MapPoint::new(632.4453, 1835.14)
    );
    assert_eq!(
        ai.base.seek_position.map_point(),
        MapPoint::new(200.0, 200.0)
    );
    assert_eq!(ai.base.when_does_timer_ring, 120);
}

fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
    crate::engine::test_support::pair_fixture::PairFixture::new(
        make_test_ai_soldier(Camp::Lacklandists),
        make_test_pc(Posture::Upright),
    )
    .grid(256, 256)
    .extent(4000.0)
    .action_state(ActionState::Waiting)
    .move_box(crate::coordinates::MoveBox::from_coords(
        -4.0, -4.0, 4.0, 4.0,
    ))
    .mission_script("observation.scs")
    .think_first()
    .build()
    .into_tuple()
}

fn place(engine: &mut EngineInner, id: EntityId, point: WorldPoint3D) {
    engine.place(id, point);
}

fn door_position(engine: &mut EngineInner, actor: EntityId, point: MapPoint) {
    let sector = engine
        .expect_entity(actor, "test door actor")
        .element_data()
        .sector()
        .unwrap();
    let door = crate::gate::DoorIndex::new(engine.script_domains.interactables.doors.len() as u32)
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: point,
            point_out: point,
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut element = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(actor),
        OrderType::WalkingUpright,
    );
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut element.data
    else {
        unreachable!()
    };
    *gate_id = Some(door);
    *direction = 1;
    let sequence = engine.orders.sequence_manager.insert_element(element);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(actor, Some((sequence, 0)));
    engine.t_element_in_progress(&LevelAssets::new(), sequence, 0);
}

#[test]
fn enemy_sighting_uses_live_geometry_without_detection_capture() {
    let (mut engine, assets, owner, target) = fixture();
    engine.execute_ai_seen_enemy(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        target.index(),
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("sighting result"));
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingReactiontimeTurning
    );
    assert!(!ai.enemy_seen_below);
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
}

#[test]
fn moving_sighting_approach_radius_uses_raw_stretched_world_distance() {
    for (owner_point, target_point, door, radius) in [
        (
            WorldPoint3D::new(1171.3004, 1846.5278, 0.0),
            WorldPoint3D::new(1648.9281, 1804.8717, 0.0),
            Some(MapPoint::new(1151.0, 1817.0)),
            161,
        ),
        (
            WorldPoint3D::new(277.4972, 379.12796 + 1.387514, 1.387514),
            WorldPoint3D::new(57.0, 245.0 + 36.001007, 36.001007),
            None,
            94,
        ),
    ] {
        let (mut engine, assets, owner, target) = fixture();
        place(&mut engine, owner, owner_point);
        place(&mut engine, target, target_point);
        if let Some(point) = door {
            door_position(&mut engine, owner, point);
        }
        engine.set_action_state_of(owner, ActionState::MovingFast);
        engine.execute_ai_seen_enemy(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            target.index(),
        );
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("moving sighting"));
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingReactiontimeRunning
        );
        assert_eq!(ai.base.stop_before_end_of_path_distance, radius);
        assert_eq!(ai.base.when_does_timer_ring, 110);
    }
}

#[test]
fn near_sighting_gate_uses_world_y_and_elevation() {
    let (mut engine, assets, owner, target) = fixture();
    place(
        &mut engine,
        owner,
        WorldPoint3D::new(575.6, 2465.001, 105.001),
    );
    place(
        &mut engine,
        target,
        WorldPoint3D::new(609.0, 2449.001, 150.001),
    );
    engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("near sighting"))
        .combat_trainer = true;
    engine.execute_ai_seen_enemy(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        target.index(),
    );
    assert_ne!(
        engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("near sighting"))
            .base
            .current_substate,
        Substate::AttackingReactiontimeTurning
    );
}

#[test]
fn near_sighting_gate_reads_raw_target_during_door_pass() {
    let (mut engine, assets, owner, target) = fixture();
    place(
        &mut engine,
        owner,
        WorldPoint3D::new(654.72314, 1403.2888 + 143.06665, 143.06665),
    );
    place(
        &mut engine,
        target,
        WorldPoint3D::new(560.9536, 1552.7451, 130.001),
    );
    door_position(&mut engine, target, MapPoint::new(663.75, 1421.5));
    engine.execute_ai_seen_enemy(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        target.index(),
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("door sighting"))
            .base
            .current_substate,
        Substate::AttackingReactiontimeTurning
    );
}

#[test]
fn shadow_changes_music_alert_without_accelerating_view_refresh() {
    let (mut engine, assets, owner, _) = fixture();
    let position = Position {
        x: 300.0,
        y: 400.0,
        ..engine.live_ai_position(owner)
    };
    engine.execute_ai_seen_shadow(&crate::sim_rng::test_context(), &assets, owner, position);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("shadow sighting"));
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
    assert_eq!(ai.base.current_substate, Substate::DefaultLookingShadow);
    assert_eq!(ai.base.when_does_timer_ring, 110);
}

#[test]
fn runtime_objects_trigger_reactions_but_bonus_variants_are_ignored() {
    for (object_type, expected) in [
        (
            ObjectType::Purse,
            Some(Substate::WonderingMoneyReactiontime),
        ),
        (ObjectType::Coin, Some(Substate::WonderingMoneyReactiontime)),
        (ObjectType::Ale, Some(Substate::WonderingAleReactiontime)),
        (ObjectType::BonusPurse, None),
        (ObjectType::BonusAle, None),
    ] {
        let (mut engine, assets, owner, _) = fixture();
        let sector = engine.live_ai_position(owner).sector;
        let mut element = ElementData::default();
        element.kind = ElementKind::ObjectOther;
        element.active = true;
        element.set_sector_topology(sector, sector.and_then(|sector| sector.arena_index()));
        element.set_position(WorldPoint3D::new(300.0, 400.0, 0.0));
        let object = engine.add_test_entity(Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                object_type,
                ..Default::default()
            },
        }));
        engine.execute_ai_seen_object(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            object.index(),
        );
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("object sighting"));
        assert_eq!(
            ai.base.current_substate,
            expected.unwrap_or(Substate::DefaultOnPost)
        );
        assert_eq!(
            ai.base.interesting_object,
            expected.map(|_| AiEntityHandle::new(object.index()))
        );
        if matches!(object_type, ObjectType::Purse | ObjectType::Coin) {
            assert_eq!(ai.base.when_does_timer_ring, 130);
        }
    }
}

#[test]
fn moving_sighting_rereads_target_after_state_callback() {
    use crate::engine::test_support::asm::*;
    use crate::engine::types::MissionScript;
    use crate::natives::{NativeFn, ScriptHandleCodec};
    use crate::scb::{ClassEntry, Function, ScbFile};
    let (mut engine, mut assets, owner, target) = fixture();
    engine.set_action_state_of(owner, ActionState::MovingFast);
    engine.actor_mut(owner).script_class = "MoveObserved".into();
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(600.0, 700.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![0]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![1]);
    assets.scripts.location_sector_handles =
        std::sync::Arc::new(vec![engine.live_ai_position(target).sector]);
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![
                empty_startup_class("sighting.scs".into()),
                ClassEntry {
                    source_file: "sighting.scs".into(),
                    class_name: "MoveObserved".into(),
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
                        q_aff1_get_param(0xC000, 4),
                        q_aff0_iconstant(0xC004, AiState::Attacking.state_change_event_code()),
                        q_ieq(0xC000, 0xC000, 0xC004),
                        q_if_not_zero_goto(0xC000, 7),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_aff0_iconstant(0xC000, ScriptHandleCodec::actor_handle(target)),
                        q_aff0_iconstant(0xC004, ScriptHandleCodec::location_handle_from_index(0)),
                        q_native_param(0xC000),
                        q_native_param(0xC004),
                        q_native_call(NativeFn::SetActorLocation as u32),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_end_function(),
                    ],
                },
            ],
        })
        .unwrap(),
    );
    engine.attach_script_bindings(&assets);
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .bind_actor(ScriptHandleCodec::actor_handle(owner), "MoveObserved");
    engine.execute_ai_seen_enemy(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        target.index(),
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("post-callback sighting"));
    assert_eq!(
        ai.base.last_goto_destination.map_point(),
        MapPoint::new(600.0, 700.0)
    );
    let stretched_y = 600.0 * crate::position_interface::INVERSE_ASPECT_RATIO;
    let expected_radius = ((500.0 * 500.0 + stretched_y * stretched_y).sqrt() / 3.0) as u16;
    assert_eq!(
        ai.base.stop_before_end_of_path_distance, expected_radius,
        "distance and destination are evaluated after the callback moves the target"
    );
    assert_eq!(
        ai.base.my_reconnaissance_report.seek_position.map_point(),
        MapPoint::new(200.0, 100.0),
        "the earlier report retains its pre-callback position"
    );
}
