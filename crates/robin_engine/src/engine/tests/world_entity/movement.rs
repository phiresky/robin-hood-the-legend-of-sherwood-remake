use super::*;

#[test]
fn owner_boundary_positions_follow_original_creation_order_not_entity_slots() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::entities::{BoundaryPosition, EntitySlots};
    use std::collections::BTreeMap;

    let mut engine = EngineInner::new();
    // Deliberately allocate in the opposite order from Original's element
    // walk. Rust slots are loader/runtime storage identities; Original
    // Update visibility is determined by creation order.
    let later_target = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let owner = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let earlier_target =
        engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    engine.world.install_original_creation_orders(
        BTreeMap::from([(later_target, 30), (owner, 20), (earlier_target, 10)]),
        31,
    );
    assert!(later_target.index() < owner.index());
    assert!(earlier_target.index() > owner.index());

    let earlier_before = BoundaryPosition {
        map: MapPoint::new(10.0, 20.0),
        world: WorldPoint3D::new(10.0, 23.0, 3.0),
    };
    let owner_before = BoundaryPosition {
        map: MapPoint::new(30.0, 40.0),
        world: WorldPoint3D::new(30.0, 45.0, 5.0),
    };
    let later_before = BoundaryPosition {
        map: MapPoint::new(50.0, 60.0),
        world: WorldPoint3D::new(50.0, 67.0, 7.0),
    };
    let mut before = EntitySlots::filled(engine.world.entities.len(), None);
    before[earlier_target] = Some(earlier_before);
    before[owner] = Some(owner_before);
    before[later_target] = Some(later_before);

    let earlier_live = WorldPoint3D::new(110.0, 123.0, 13.0);
    let owner_live = WorldPoint3D::new(130.0, 145.0, 15.0);
    let later_live = WorldPoint3D::new(150.0, 167.0, 17.0);
    engine
        .get_entity_mut(earlier_target)
        .unwrap()
        .element_data_mut()
        .set_position(earlier_live);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_position(owner_live);
    engine
        .get_entity_mut(later_target)
        .unwrap()
        .element_data_mut()
        .set_position(later_live);

    assert_eq!(
        engine.boundary_position(
            earlier_target,
            owner,
            &before,
            crate::engine::ai::OwnerActorPhase::AfterActor
        ),
        BoundaryPosition::of(engine.get_entity(earlier_target).unwrap().element_data()),
        "an earlier Original slot has already completed its actor movement"
    );
    assert_eq!(
        engine.boundary_position(
            later_target,
            owner,
            &before,
            crate::engine::ai::OwnerActorPhase::AfterActor
        ),
        later_before,
        "a later Original slot still exposes its preserved pre-movement position"
    );
    assert_eq!(
        engine.boundary_position(
            owner,
            owner,
            &before,
            crate::engine::ai::OwnerActorPhase::BeforeActor
        ),
        owner_before,
        "the owner itself is pre-movement before its actor-update phase"
    );
    assert_eq!(
        engine.boundary_position(
            owner,
            owner,
            &before,
            crate::engine::ai::OwnerActorPhase::AfterActor
        ),
        BoundaryPosition::of(engine.get_entity(owner).unwrap().element_data()),
        "the owner itself is live after its actor-update phase"
    );
}

#[test]
fn typed_route_continuation_keeps_end_think_open_for_its_fallback_move() {
    use crate::ai::{StimulusType, Substate};
    use crate::element::AiBrain;

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(owner)
        .expect("typed-continuation test soldier exists")
    else {
        unreachable!()
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());

    let ai = soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("typed-continuation test soldier has AI");
    ai.current_substate = Substate::SeekingBodyLookingDeadBody;
    ai.think_recursion_depth = 1;
    ai.completion_latch_inside_think = true;
    // Dead-body alerting moved its first approach into an ActorEffects owner-work
    // prefix. The following typed tail will consume that verdict and may
    // author fallback area-seeking movement before the same tick completion returns.
    ai.outbox.reentrant.dead_body_alert_completion_pending = true;
    assert!(ai.end_think_completion_events());
    assert_eq!(ai.think_recursion_depth, 1);
    assert_eq!(ai.engine_deferred_end_think_frames, 1);

    // Model the typed tail consuming its first failure, then its fallback
    // movement failing synchronously. That second verdict still belongs to
    // the original open Think and must recurse into the seek handler.
    ai.outbox.reentrant.dead_body_alert_completion_pending = false;
    ai.couldnt_reachpoint = true;
    engine.surface_synchronous_completion_events_for_owner(owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("typed-continuation test soldier retains AI");
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventCouldntReachPoint]
    );
}

#[test]
fn attentive_barrier_constructs_following_move_at_same_owner_boundary() {
    use crate::ai::AttentiveModeEffect;
    use crate::element::{AiBrain, Command, Posture};
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = crate::coordinates::MapSize::new(500.0, 500.0);
    let mut soldier_entity = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let enemy = soldier.npc.ai_brain.enemy_mut().expect("Enemy test AI");
    enemy.attentive = true;
    enemy.will_be_attentive = true;
    enemy
        .base
        .outbox
        .actor
        .queue_set_attentive_mode(AttentiveModeEffect::new(false, false));
    let mut movement = AiOrderIntent::new(OrderType::WalkingUpright, 100.0, 90.0);
    movement.after_attentive_mode = true;
    enemy.base.outbox.actor.orders.push(movement);
    let owner = engine.add_test_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_direct_ai_owner_boundary_mode(
        &sim,
        owner,
        &assets,
        crate::engine::ai::OwnerBoundaryPolicy::WithoutForecast,
    );

    assert!(
        engine.orders.pending_move_requests.is_empty(),
        "movement following an attentive-mode change must construct inline, not wait for the global movement drain"
    );
    let commands = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| element.command)
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        [Command::LeaveAttentiveMode, Command::Move],
        "attentive registration stays first while movement construction remains synchronous"
    );
}

#[test]
fn stop_exclamation_removes_only_first_same_actor_request_in_each_sound_phase() {
    use crate::sound::{
        ExclamationGroup, PendingExclamation, PlayingExclamation, ResolvedExclamation,
    };

    let pending = |actor_id, profile_id, exclamation_id| PendingExclamation {
        actor_id,
        group: ExclamationGroup::Civilian,
        profile_id,
        exclamation_id,
        variant: -1,
    };
    let resolved = |actor_id, profile_id: u32, exclamation_id| ResolvedExclamation {
        actor_id,
        identifier: (profile_id & 0xFFFF_0000) | u32::from(exclamation_id),
        exclamation_id,
        duration_frames: 10,
    };

    let mut unresolved = EngineInner::new();
    unresolved.feedback.sound_sim.pending_exclamations = vec![
        pending(7, 0x1111_0000, 1),
        pending(8, 0x2222_0000, 2),
        pending(7, 0x3333_0000, 3),
    ];
    unresolved.feedback.sound_sim.resolved_exclamations = vec![
        resolved(7, 0x1111_0000, 1),
        resolved(8, 0x2222_0000, 2),
        resolved(7, 0x3333_0000, 3),
    ];

    unresolved.cancel_exclamation_callbacks(7);

    assert_eq!(
        unresolved
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|request| (request.actor_id, request.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(8, 2), (7, 3)]
    );
    assert_eq!(
        unresolved
            .feedback
            .sound_sim
            .resolved_exclamations
            .iter()
            .map(|request| (request.actor_id, request.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(8, 2), (7, 3)]
    );

    let mut playing = EngineInner::new();
    playing.feedback.sound_sim.playing_exclamations = vec![
        PlayingExclamation {
            actor_id: 7,
            exclamation_id: 1,
            finish_frame: 10,
        },
        PlayingExclamation {
            actor_id: 8,
            exclamation_id: 2,
            finish_frame: 11,
        },
        PlayingExclamation {
            actor_id: 7,
            exclamation_id: 3,
            finish_frame: 12,
        },
    ];
    playing.feedback.sound_sim.pending_exclamations = vec![pending(7, 0x4444_0000, 4)];
    playing.feedback.sound_sim.finished_exclamations = vec![(7, 0), (8, 5)];

    playing.cancel_exclamation_callbacks(7);

    assert_eq!(
        playing
            .feedback
            .sound_sim
            .playing_exclamations
            .iter()
            .map(|request| (request.actor_id, request.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(8, 2), (7, 3)]
    );
    assert_eq!(
        playing
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|request| (request.actor_id, request.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(7, 4)],
        "the later unresolved request is a distinct Original pending node"
    );
    assert_eq!(
        playing.feedback.sound_sim.finished_exclamations,
        vec![(7, 0), (8, 5)],
        "StopExclamation cannot retract an already-delivered completion"
    );
}

#[test]
fn unrelated_running_to_officer_failure_remains_generic() {
    use crate::ai::{AiState, StimulusType, Substate};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("generic route-failure owner has Enemy AI");
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingRunningToOfficer;
    ai.base.completion_latch_inside_think = true;
    ai.base.couldnt_reachpoint = true;

    engine.surface_synchronous_completion_events_for_owner(owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("generic route-failure owner retains Enemy AI");
    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(
        ai.base.outbox.reentrant.self_stimuli,
        vec![StimulusType::EventCouldntReachPoint]
    );
    assert!(ai.seek_flags.is_empty());
    assert!(ai.personal_seek_point_2.is_none());
}

#[test]
fn entity_building_sector_uses_exact_identity_before_public_number_fallback() {
    let mut engine = EngineInner::new();
    let public = crate::sector::SectorNumber::new(88);
    let make_sector = |sector_type| crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: MapBBox::new(),
        sector_type,
        layer: 0,
        sector_number: public,
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };

    let mut level = crate::fast_find_grid::LevelGrid::default();
    level
        .sectors
        .push(make_sector(crate::sector::SectorType::BUILDING));
    level
        .sectors
        .push(make_sector(crate::sector::SectorType::empty()));
    level.sector_number_map.insert(public, 0);
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);

    let number_only = crate::position_interface::SectorHandle::new(88).unwrap();
    let exact_building = number_only.with_arena_index(
        crate::fast_find_grid::SectorIndex::new(0).expect("building arena index"),
    );
    let exact_ordinary = number_only.with_arena_index(
        crate::fast_find_grid::SectorIndex::new(1).expect("ordinary arena index"),
    );

    assert_eq!(
        engine.entity_building_sector(exact_building.into()),
        Some(exact_building),
        "the original game checks the actor's exact sector identity when finding its building"
    );
    assert_eq!(
        engine.entity_building_sector(exact_ordinary.into()),
        None,
        "an outside duplicate-public sector must stay visible to outside observers"
    );
    assert_eq!(
        engine.entity_building_sector(number_only.into()),
        Some(number_only),
        "identity-less compatibility positions retain the public-number lookup"
    );
}

#[test]
fn selection_mark_skips_hidden_and_building_pcs() {
    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    engine.players.seats[0].selection.push(pc_id);

    assert!(engine.pc_draws_selection_mark(pc_id));
    assert!(engine.any_selected_pc_drawing_selection_mark());

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.hidden_in_building = true;
    }
    assert!(!engine.pc_draws_selection_mark(pc_id));
    assert!(!engine.any_selected_pc_drawing_selection_mark());

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.hidden_in_building = false;
    }

    let sector_num = crate::position_interface::SectorHandle::new(42).unwrap();
    install_test_building_sector(&mut engine, 42);

    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.set_sector(Some(sector_num));
    }

    assert!(!engine.pc_draws_selection_mark(pc_id));
    assert!(!engine.any_selected_pc_drawing_selection_mark());
}

#[test]
fn friend_swap_candidates_resolve_both_friend_and_target_through_ai_position() {
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(friend_soldier) = engine.get_entity_mut(friend).unwrap() else {
        panic!("friend changed kind")
    };
    friend_soldier
        .element
        .set_position_map(MapPoint::new(11.0, 12.0));
    assert!(friend_soldier.actor.active_door_pass.is_none());
    // A null live sprite door must not suppress the selected movement gate.
    assert!(
        friend_soldier
            .element
            .sprite
            .position_iface
            .get_door()
            .is_none()
    );
    let friend_ai = friend_soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("friend has AI");
    friend_ai.current_substate = crate::ai::Substate::AttackingRunningToEnemy;
    friend_ai.primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));

    let Entity::Pc(target_pc) = engine.get_entity_mut(target).unwrap() else {
        panic!("target changed kind")
    };
    target_pc
        .element
        .set_position_map(MapPoint::new(21.0, 22.0));
    assert!(target_pc.actor.active_door_pass.is_none());
    // A different live sprite door must not replace the selected movement
    // element's gate or direction for AI Position.
    target_pc.element.sprite.position_iface.set_door(
        crate::position_interface::DoorHandle::new(2).expect("valid door index"),
        true,
    );

    for (passing, gate, direction) in [
        (friend, DoorIndex::new(0).expect("valid door index"), 1),
        (target, DoorIndex::new(1).expect("valid door index"), 0),
    ] {
        let mut element = SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(passing),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            gate_id,
            direction: movement_direction,
            ..
        } = &mut element.data
        else {
            panic!("PassDoor test element changed kind")
        };
        *gate_id = Some(gate);
        *movement_direction = direction;
        let sequence_id = engine.orders.sequence_manager.launch_element(element);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
    }

    engine.script_domains.interactables.doors = vec![
        Door {
            point_in: MapPoint::new(101.0, 102.0),
            sector_in: SectorNumber::new(11),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(11),
            layer_in: 3,
            ..Door::default()
        },
        Door {
            point_out: MapPoint::new(201.0, 202.0),
            sector_out: SectorNumber::new(22),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(22),
            layer_out: 4,
            ..Door::default()
        },
        Door {
            point_in: MapPoint::new(301.0, 302.0),
            sector_in: SectorNumber::new(33),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(33),
            layer_in: 5,
            ..Door::default()
        },
    ];

    let candidates = crate::engine::ai::build_friend_swap_candidates(
        &engine.world.entities,
        &engine.mission_domain.diplomacy,
        &engine.script_domains.interactables.doors,
        &engine.orders.sequence_manager,
        owner,
        crate::element::Camp::Lacklandists,
        |element| crate::engine::ai::ai_view_position_sector(&engine, element),
    );
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.friend_id, friend);
    assert_eq!(candidate.friend_position.x, 101.0);
    assert_eq!(candidate.friend_position.y, 102.0);
    assert_eq!(
        candidate.friend_position.sector,
        crate::position_interface::SectorHandle::new(11)
    );
    assert_eq!(candidate.friend_position.level, 3);
    assert_eq!(
        candidate.friend_primary_target,
        Some(crate::ai::AiEntityHandle::new(target.index()))
    );
    assert_eq!(candidate.friend_primary_target_position.x, 201.0);
    assert_eq!(candidate.friend_primary_target_position.y, 202.0);
    assert_eq!(
        candidate.friend_primary_target_position.sector,
        crate::position_interface::SectorHandle::new(22)
    );
    assert_eq!(candidate.friend_primary_target_position.level, 4);
}

#[test]
fn friend_swap_candidate_preserves_exact_duplicate_target_sector() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::GridSector;
    use crate::sector::{SectorNumber, SectorType};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let square = |min: f32, max: f32| GridSector {
        points: vec![
            MapPoint::new(min, min),
            MapPoint::new(max, min),
            MapPoint::new(max, max),
            MapPoint::new(min, max),
        ],
        bounding_box: MapBBox::from_coords(min, min, max, max),
        sector_number: SectorNumber::new(88),
        layer: 2,
        sector_type: SectorType::MOTION,
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };
    let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
    let level = std::sync::Arc::make_mut(&mut grid.level);
    level.sectors = vec![square(0.0, 100.0), square(600.0, 800.0)];

    let Entity::Soldier(friend_soldier) = engine.get_entity_mut(friend).unwrap() else {
        panic!("friend changed kind")
    };
    friend_soldier
        .npc
        .ai_brain
        .base_mut()
        .unwrap()
        .current_substate = crate::ai::Substate::AttackingRunningToEnemy;
    friend_soldier
        .npc
        .ai_brain
        .base_mut()
        .unwrap()
        .primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));

    let target_element = engine.get_entity_mut(target).unwrap().element_data_mut();
    target_element.set_position_map(MapPoint::new(684.1841, 745.0576));
    target_element.set_layer(2);
    target_element.set_sector(crate::position_interface::SectorHandle::new(88));

    let candidates = crate::engine::ai::build_friend_swap_candidates(
        &engine.world.entities,
        &engine.mission_domain.diplomacy,
        &[],
        &engine.orders.sequence_manager,
        owner,
        crate::element::Camp::Lacklandists,
        |element| crate::engine::ai::ai_view_position_sector(&engine, element),
    );

    let target_sector = candidates[0]
        .friend_primary_target_position
        .sector
        .expect("friend target must retain its sector");
    assert_eq!(u16::from(target_sector), 88);
    assert_eq!(
        target_sector.arena_index(),
        crate::fast_find_grid::SectorIndex::new(1)
    );
}

#[test]
fn ai_position_ignores_misassociated_pass_door_for_non_actor() {
    use crate::coordinates::MapPoint;
    use crate::element::{ElementBonus, ElementData, ElementKind, ObjectData, ObjectType};
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let object_id = engine.add_test_entity(Entity::Bonus(ElementBonus {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectBonus;
            initial_element.active = true;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Coin,
            ..ObjectData::default()
        },
    }));

    let mut pass_door = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(object_id),
        OrderType::WalkingUpright,
    );
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass_door.data
    else {
        panic!("PassDoor test element changed kind")
    };
    *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
    *direction = 1;
    let sequence_id = engine.orders.sequence_manager.launch_element(pass_door);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let doors = [Door {
        point_in: MapPoint::new(101.0, 102.0),
        ..Door::default()
    }];
    let raw = crate::ai::Position {
        x: 11.0,
        y: 12.0,
        sector: crate::position_interface::SectorHandle::new(3),
        level: 4,
    };
    let resolved = crate::engine::ai::resolve_ai_position_with(
        &engine.world.entities,
        &doors,
        &engine.orders.sequence_manager,
        object_id,
        |_| raw,
    );
    assert_eq!(resolved.effective.x, raw.x);
    assert_eq!(resolved.effective.y, raw.y);
    assert_eq!(resolved.effective.sector, raw.sector);
    assert_eq!(resolved.effective.level, raw.level);
}

#[test]
fn avenger_roof_wait_uses_selected_pass_door_position_and_preserves_ordinary_fallback() {
    use crate::ai::{AiContext, AiState, Position, Substate};
    use crate::coordinates::MapPoint;
    use crate::fast_find_grid::GridSector;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::{SectorNumber, SectorType};
    use crate::sequence::{SequenceElement, SequenceElementData};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    {
        let level = engine.world.fast_grid_mut().level_mut();
        level.sectors = (0..=2)
            .map(|number| GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::MOTION | SectorType::AREA,
                layer: 0,
                sector_number: SectorNumber::new(number),
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            })
            .collect();
        level.sector_number_map = (0..=2)
            .map(|number| (SectorNumber::new(number), number as usize))
            .collect();
    }
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "pending_lift_roof_wait_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission enables roof-wait tick construction"),
    );
    let owner_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let me_sector = crate::position_interface::SectorHandle::new(1);

    for (id, position) in [
        (owner_id, MapPoint::new(100.0, 0.0)),
        (target_id, MapPoint::new(100.0, 25.0)),
    ] {
        let entity = engine.get_entity_mut(id).expect("roof-wait actor exists");
        entity.element_data_mut().active = true;
        entity.element_data_mut().set_position_map(position);
        // Schema-12 actors can retain only the public sector number even
        // though the loaded gate graph has exact arena identities. Original
        // Actor positioning still supplies the exact sector reference to the roof
        // fallback lookup, so exercise the runtime recovery path here.
        entity.element_data_mut().set_sector(me_sector);
        assert_eq!(entity.element_data().sector().unwrap().arena_index(), None);
    }
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("roof-wait owner has Enemy AI");
    owner.base.me = owner_id.index();
    owner.base.primary_target = Some(crate::ai::AiEntityHandle::new(target_id.index()));

    // The target sprite is still on the owner's side. The original game's AI position
    // instead reports point_in/sector_in while this PassDoor is selected.
    let mut door = Door {
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(2),
        point_out: MapPoint::new(100.0, 100.0),
        point_in: MapPoint::new(100.0, 200.0),
        ..Door::default()
    };
    door.lock_npc_villain();
    engine.script_domains.interactables.doors = vec![door];

    let mut pass = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(target_id),
        OrderType::WalkingUpright,
    );
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    else {
        panic!("PassDoor test element changed kind")
    };
    *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
    *direction = 1;
    let sequence_id = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let wait = crate::engine::ai::precompute_avenger_on_roof_wait_position(
        &engine.world.entities,
        &engine.script_domains.interactables.doors,
        &engine.orders.sequence_manager,
        owner_id,
        target_id,
        |element| crate::engine::ai::ai_view_position_sector(&engine, element),
        &|_| true,
        &|_| None,
    )
    .expect("committed target side exposes the NPC-locked blocking gate");
    assert_eq!(wait.x, 100.0);
    assert_eq!(wait.y, 100.0);
    assert_eq!(wait.sector, me_sector);

    // The staged lift failure has not published couldnt_reachpoint when its
    // EventCouldnt tick is built. The exact RunningToLadder timer provenance
    // must still make the live gate lookup available to the handler.
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("roof-wait owner retains Enemy AI");
    owner.base.current_state = AiState::Attacking;
    owner.base.current_substate = Substate::AttackingRunningToLadder;
    owner.base.couldnt_reachpoint = false;
    owner.base.timer_is_running = true;
    owner.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
    owner.base.when_does_timer_ring = 30;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);
    assert_eq!(
        tick.avenger_wait_position_for(target_id.index()),
        Some(wait),
        "pending lift provenance must precompute the source-synchronous roof wait"
    );

    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("roof-wait owner retains Enemy AI");
    owner.base.current_state = AiState::Attacking;
    owner.base.current_substate = Substate::AttackingRunningToEnemy;
    owner.base.couldnt_reachpoint = true;
    owner.resume_reconsider_enemy_approach_after_go_near(
        Position {
            x: 100.0,
            y: 200.0,
            sector: crate::position_interface::SectorHandle::new(2),
            level: 0,
        },
        Some(wait),
        &AiContext::test_fixture(),
    );
    assert!(!owner.base.couldnt_reachpoint);
    assert_eq!(
        owner.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(owner.base.outbox.actor.orders.len(), 1);
    assert_eq!(owner.base.outbox.actor.orders[0].target_x, 100.0);
    assert_eq!(owner.base.outbox.actor.orders[0].target_y, 100.0);
    assert!(
        !owner.base.outbox.actor.orders[0].defer_instruction,
        "an ordinary route failure registers before this frame's manager boundary"
    );

    engine
        .orders
        .sequence_manager
        .element_terminated(sequence_id, 0);

    // Result616 reaches the same lookup without a selected PassDoor: both
    // ordinary actor positions came from the legacy save as number-only
    // handles while every loaded gate endpoint was exact. Recover both
    // pointers before starting the identity-aware path walk.
    {
        let target = engine
            .get_entity_mut(target_id)
            .expect("roof-wait target remains live");
        target
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 200.0));
        target
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(2));
        assert_eq!(target.element_data().sector().unwrap().arena_index(), None);
    }
    let ordinary_wait = crate::engine::ai::precompute_avenger_on_roof_wait_position(
        &engine.world.entities,
        &engine.script_domains.interactables.doors,
        &engine.orders.sequence_manager,
        owner_id,
        target_id,
        |element| crate::engine::ai::ai_view_position_sector(&engine, element),
        &|_| true,
        &|_| None,
    )
    .expect("ordinary restored positions recover their exact gate sectors");
    assert_eq!(ordinary_wait, wait);

    // Without the selected PassDoor, both ordinary live positions are in the
    // same sector. The roof special case must remain absent so the caller can
    // retain couldn't-reachpoint and take its normal emergency fallback.
    engine
        .get_entity_mut(target_id)
        .expect("roof-wait target remains live")
        .element_data_mut()
        .set_sector(me_sector);
    assert!(
        crate::engine::ai::precompute_avenger_on_roof_wait_position(
            &engine.world.entities,
            &engine.script_domains.interactables.doors,
            &engine.orders.sequence_manager,
            owner_id,
            target_id,
            |element| crate::engine::ai::ai_view_position_sector(&engine, element),
            &|_| true,
            &|_| None,
        )
        .is_none()
    );

    // Same false-latch timer provenance with no blocking gate is the f7938
    // no-roof control: no synthetic wait may be added.
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("roof-wait owner retains Enemy AI");
    owner.base.current_state = AiState::Attacking;
    owner.base.current_substate = Substate::AttackingRunningToLadder;
    owner.base.couldnt_reachpoint = false;
    owner.base.timer_is_running = true;
    owner.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
    owner.base.when_does_timer_ring = 30;
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);
    assert!(
        tick.avenger_wait_position_for(target_id.index()).is_none(),
        "pending lift provenance must preserve the no-blocking-gate control"
    );
}

#[test]
fn seek_area_friend_scan_uses_selected_pass_door_without_runtime_latch() {
    use crate::ai::{AlertLevel, Substate};
    use crate::ai_enemy::SeekFlags;
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    // Preserve Original's null AI-handle slot.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let owner_position = MapPoint::new(1155.7197, 1421.6211);
    let friend_raw_position = MapPoint::new(727.0, 1168.0);

    for (id, position) in [(owner_id, owner_position), (friend_id, friend_raw_position)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test soldier exists")
        else {
            panic!("test soldier changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(position);
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI")
            .base
            .me = id.index();
    }
    let friend = engine
        .get_entity_mut(friend_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("friend has enemy AI");
    friend.base.view_alert_status = AlertLevel::Yellow;
    friend.base.current_substate = Substate::SeekingSeekpoint;
    friend.seek_flags.insert(SeekFlags::LOOK_FOR_HELP_AFTER);

    engine.script_domains.interactables.doors = vec![Door {
        point_out: MapPoint::new(718.0, 1179.0),
        point_in: MapPoint::new(735.0, 1156.0),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(0),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(1),
        ..Door::default()
    }];
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "seek_area_selected_pass_door_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed test door"),
    );
    let mut pass = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(friend_id),
        OrderType::WalkingUpright,
    );
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    else {
        panic!("PassDoor test element changed kind")
    };
    *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
    *direction = 0;
    let sequence_id = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    assert!(
        engine
            .get_entity(friend_id)
            .expect("friend exists")
            .actor_data()
            .expect("friend is actor")
            .active_door_pass
            .is_none(),
        "fixture must model a selected legacy PassDoor without a runtime choreography latch"
    );

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);
    assert_eq!(tick.visible_seeking_friends, 0);
    assert!(!tick.friend_seek_clears_help_flag);
    let tick_owner = tick
        .owner_live_position
        .expect("owner position is populated");
    assert_eq!(tick_owner.x, owner_position.x);
    assert_eq!(tick_owner.y, owner_position.y);

    engine
        .orders
        .sequence_manager
        .element_terminated(sequence_id, 0);
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);
    assert_eq!(tick.visible_seeking_friends, 1);
    assert!(tick.friend_seek_clears_help_flag);
}

#[test]
fn optical_ai_position_uses_carrier_boundary_but_detects_the_target_world_point() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let carrier = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::OnShoulders));

    let carrier_world = WorldPoint3D::new(321.25, 654.5, 11.0);
    let Entity::Pc(carrier_pc) = engine.get_entity_mut(carrier).expect("carrier PC exists") else {
        panic!("carrier changed kind")
    };
    carrier_pc.element.active = true;
    carrier_pc.pc.life_points = 100;
    carrier_pc.element.set_position(carrier_world);
    carrier_pc
        .element
        .set_position_map(MapPoint::new(321.25, 640.0));

    let exact_target_world = WorldPoint3D::new(12.345_679, 98.765_434, 7.654_321);
    let Entity::Pc(target_pc) = engine.get_entity_mut(target).expect("carried PC exists") else {
        panic!("carried target changed kind")
    };
    target_pc.element.active = true;
    target_pc.pc.life_points = 100;
    target_pc.human.carrier = Some(carrier);
    target_pc.element.set_position_map(MapPoint::from_world_xyz(
        exact_target_world.x,
        exact_target_world.y,
        exact_target_world.z,
    ));
    target_pc.element.set_position(exact_target_world);
    let expected_optical_point = crate::stealth::detection_point_world(
        exact_target_world,
        target_pc.element.posture(),
        target_pc.element.direction(),
        false,
    );

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = engine.boundary_positions_snapshot();
    let Entity::Pc(carrier_pc) = engine.get_entity_mut(carrier).expect("carrier PC remains") else {
        panic!("carrier changed kind")
    };
    carrier_pc
        .element
        .set_position(WorldPoint3D::new(999.0, 999.0, 99.0));
    carrier_pc
        .element
        .set_position_map(MapPoint::new(999.0, 999.0));

    let (ai_position, optical_point) =
        engine.enemy_optical_geometry_at_owner_for_test(&assets, owner, &positions, target);
    assert_eq!(ai_position.x, 321.25);
    assert_eq!(ai_position.y, 640.0);
    assert_eq!(
        optical_point.x.to_bits(),
        expected_optical_point.x.to_bits()
    );
    assert_eq!(
        optical_point.y.to_bits(),
        expected_optical_point.y.to_bits()
    );
    assert_eq!(
        optical_point.z.to_bits(),
        expected_optical_point.z.to_bits()
    );
}

#[test]
fn review2_instruct_gather_position_closes_at_owner_boundary() {
    use crate::ai::{CrossNpcAction, Position};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let gather = Position {
        x: 55.0,
        y: 12.0,
        ..Default::default()
    };
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 gather source has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::InstructGatherPosition {
            target: soldier_id.index(),
            position: gather,
            direction: 7,
            call_instruction: false,
        });

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 gather target retains EnemyAi");
    assert_eq!(soldier.gather_position, gather);
    assert_eq!(soldier.gather_direction, 7);
    assert!(soldier.gather_position_instructed);
    assert!(
        !engine
            .get_entity(officer_id)
            .and_then(Entity::ai_controller)
            .expect("review2 gather source retains AI")
            .has_pending_synchronous_cross_npc_actions()
    );
}

#[test]
fn ai_entity_views_keep_inactive_humans_for_same_building_detection() {
    let mut engine = EngineInner::new();
    let soldier_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("inactive snapshot soldier exists")
    else {
        panic!("inactive snapshot entity changed kind")
    };
    soldier.element.active = false;

    let scratch = engine.build_sim_scratch(&LevelAssets::new());
    let view = scratch
        .ai_entity_views
        .get(&soldier_id.index())
        .expect("inactive human must remain available to same-building detection");
    assert!(!view.active);
}
