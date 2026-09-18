use super::*;

#[test]
fn owner_walk_observes_live_geometry_in_original_creation_order() {
    use crate::coordinates::MapPoint;
    use std::collections::BTreeMap;
    let mut engine = EngineInner::new();
    let later = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let owner = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let earlier = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    engine.world.install_original_creation_orders(
        BTreeMap::from([(later, 30), (owner, 20), (earlier, 10)]),
        31,
    );
    let assets = engine.test_runtime_assets();
    for (id, x) in [(earlier, 10.0), (owner, 30.0), (later, 50.0)] {
        let entity = engine.ent_mut(id);
        entity.element_data_mut().active = true;
        entity.npc_data_mut().unwrap().life_points = 100;
        engine.place_map(id, MapPoint::new(x, 0.0));
    }
    crate::ai_vision::focus_entity(engine.npc_mut(owner), later);
    let mut visits = Vec::new();
    let mut observed = None;
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &crate::sim_rng::test_context(),
        &assets,
        |engine, id| {
            visits.push(id);
            if id == earlier {
                engine.place_map(earlier, MapPoint::new(110.0, 0.0));
                // A callback may move a later actor before that actor's turn.
                engine.place_map(later, MapPoint::new(70.0, 0.0));
            } else if id == owner {
                observed = Some((engine.map_pos_of(earlier).x, engine.map_pos_of(later).x));
            } else if id == later {
                engine.place_map(later, MapPoint::new(150.0, 0.0));
            }
        },
    );
    assert_eq!(visits, vec![earlier, owner, later]);
    assert_eq!(observed, Some((110.0, 70.0)));
    assert_eq!(
        engine.npc(owner).stare_point.x,
        70.0,
        "view refresh must retain an earlier callback's mutation of a later actor"
    );
}

#[test]
fn attentive_barrier_constructs_following_move_at_same_owner_boundary() {
    use crate::element::{AiBrain, Command, Posture};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = crate::coordinates::MapSize::new(500.0, 500.0);
    engine.world.fast_grid_mut().size_map(32, 32);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        crate::engine::test_support::square_sector(
            1,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(500.0, 500.0),
        ),
        0,
    );
    let sector = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let mut soldier_entity = make_test_soldier(Posture::Upright);
    soldier_entity.element_data_mut().set_sector(Some(sector));
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let enemy = soldier.npc.ai_brain.enemy_mut().expect("Enemy test AI");
    enemy.attentive = true;
    enemy.will_be_attentive = true;
    let owner = engine.add_test_entity(soldier_entity);
    let assets = engine.test_runtime_assets();

    engine.set_soldier_attentive_mode(&sim, &assets, owner, false, false);
    let mut destination = engine.live_ai_position(owner);
    destination.x = 100.0;
    destination.y = 90.0;
    engine.duty_go_to(
        &sim,
        &assets,
        owner,
        destination,
        crate::ai::GotoFlags::empty(),
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
fn entity_building_sector_uses_exact_identity_before_public_number_fallback() {
    let mut engine = EngineInner::new();
    let public = crate::sector::SectorNumber::new(88);
    let make_sector = |sector_type| crate::fast_find_grid::GridSector {
        bounding_box: MapBBox::new(),
        sector_type,
        sector_number: public,
        ..Default::default()
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
fn live_positions_resolve_both_friend_and_target_through_selected_doors() {
    let assets = LevelAssets::new();
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(friend_soldier) = engine.ent_mut(friend) else {
        panic!("friend changed kind")
    };
    friend_soldier
        .element
        .set_position_map(MapPoint::new(11.0, 12.0));
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

    let Entity::Pc(target_pc) = engine.ent_mut(target) else {
        panic!("target changed kind")
    };
    target_pc
        .element
        .set_position_map(MapPoint::new(21.0, 22.0));
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
        let sequence_id = engine.orders.sequence_manager.insert_element(element);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence_id);
        engine.select_sequence_element(passing, Some((sequence_id, 0)));
        engine.t_element_in_progress(&assets, sequence_id, 0);
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

    let friend_position = engine.live_ai_position(friend);
    let target_position = engine.live_ai_position(target);
    assert_eq!(friend_position.x, 101.0);
    assert_eq!(friend_position.y, 102.0);
    assert_eq!(
        friend_position.sector,
        crate::position_interface::SectorHandle::new(11)
    );
    assert_eq!(friend_position.level, 3);
    assert_eq!(target_position.x, 201.0);
    assert_eq!(target_position.y, 202.0);
    assert_eq!(
        target_position.sector,
        crate::position_interface::SectorHandle::new(22)
    );
    assert_eq!(target_position.level, 4);
}

#[test]
fn live_ai_position_preserves_exact_duplicate_target_sector() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::GridSector;
    use crate::sector::{SectorNumber, SectorType};

    let mut engine = EngineInner::new();
    engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
        ..Default::default()
    };
    let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
    let level = std::sync::Arc::make_mut(&mut grid.level);
    level.sectors = vec![square(0.0, 100.0), square(600.0, 800.0)];

    let Entity::Soldier(friend_soldier) = engine.ent_mut(friend) else {
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

    let target_element = engine.elem_mut(target);
    target_element.set_position_map(MapPoint::new(684.1841, 745.0576));
    target_element.set_layer(2);
    target_element.set_sector(crate::position_interface::SectorHandle::new(88));

    let target_sector = engine
        .live_ai_position(target)
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
    let assets = LevelAssets::new();
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
    let sequence_id = engine.orders.sequence_manager.insert_element(pass_door);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence_id);
    engine.t_element_in_progress(&assets, sequence_id, 0);

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
    assert_eq!(resolved.x, raw.x);
    assert_eq!(resolved.y, raw.y);
    assert_eq!(resolved.sector, raw.sector);
    assert_eq!(resolved.level, raw.level);
}

#[test]
fn avenger_roof_wait_uses_selected_pass_door_position_and_preserves_ordinary_fallback() {
    let assets = LevelAssets::new();
    use crate::coordinates::MapPoint;
    use crate::fast_find_grid::GridSector;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::{SectorNumber, SectorType};
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    {
        let level = engine.world.fast_grid_mut().level_mut();
        level.sectors = (0..=2)
            .map(|number| GridSector {
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::MOTION | SectorType::AREA,
                sector_number: SectorNumber::new(number),
                ..Default::default()
            })
            .collect();
        level.sector_number_map = (0..=2)
            .map(|number| (SectorNumber::new(number), number as usize))
            .collect();
    }
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::engine::test_support::asm::empty_startup_class(
                "pending_lift_roof_wait_test.scs".into(),
            )],
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
        let entity = engine.ent_mut(id);
        entity.element_data_mut().active = true;
        entity.element_data_mut().set_position_map(position);
        // Schema-12 actors can retain only the public sector number even
        // though the loaded gate graph has exact arena identities. Original
        // Actor positioning still supplies the exact sector reference to the roof
        // fallback lookup, so exercise the runtime recovery path here.
        entity.element_data_mut().set_sector(me_sector);
        assert_eq!(entity.element_data().sector().unwrap().arena_index(), None);
    }
    let owner = engine.enemy_mut(owner_id);
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
    let sequence_id = engine.orders.sequence_manager.insert_element(pass);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence_id);
    engine.select_sequence_element(target_id, Some((sequence_id, 0)));
    engine.t_element_in_progress(&assets, sequence_id, 0);

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

    engine.t_element_terminated(&assets, sequence_id, 0);

    // Result616 reaches the same lookup without a selected PassDoor: both
    // ordinary actor positions came from the legacy save as number-only
    // handles while every loaded gate endpoint was exact. Recover both
    // pointers before starting the identity-aware path walk.
    {
        let target = engine.ent_mut(target_id);
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
    engine.elem_mut(target_id).set_sector(me_sector);
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
}

#[test]
fn seek_area_friend_scan_uses_selected_pass_door_without_runtime_latch() {
    let assets = LevelAssets::new();
    use crate::ai::{AlertLevel, Substate};
    use crate::ai_enemy::SeekFlags;
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData};

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
        let Entity::Soldier(soldier) = engine.ent_mut(id) else {
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
    let friend = engine.enemy_mut(friend_id);
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
            classes: vec![crate::engine::test_support::asm::empty_startup_class(
                "seek_area_selected_pass_door_test.scs".into(),
            )],
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
    let sequence_id = engine.orders.sequence_manager.insert_element(pass);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence_id);
    engine.select_sequence_element(friend_id, Some((sequence_id, 0)));
    engine.t_element_in_progress(&assets, sequence_id, 0);

    let assets = engine.test_runtime_assets();
    let live_owner = engine.live_ai_position(owner_id).map_point();
    assert_eq!(live_owner, owner_position);
    let live_friend = engine.live_ai_position(friend_id).map_point();
    assert_eq!(live_friend, MapPoint::new(718.0, 1179.0));
    let distance = live_friend - live_owner;
    assert!(distance.x * distance.x + distance.y * distance.y >= 500.0 * 500.0);

    engine.t_element_terminated(&assets, sequence_id, 0);
    let live_friend = engine.live_ai_position(friend_id).map_point();
    assert_eq!(live_friend, friend_raw_position);
    let distance = live_friend - live_owner;
    assert!(distance.x * distance.x + distance.y * distance.y < 500.0 * 500.0);
}

#[test]
fn optical_ai_position_follows_carrier_but_detects_target_stored_world_point() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    let mut engine = EngineInner::new();
    let carrier = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::OnShoulders));

    let carrier_world = WorldPoint3D::new(321.25, 654.5, 11.0);
    let Entity::Pc(carrier_pc) = engine.ent_mut(carrier) else {
        panic!("carrier changed kind")
    };
    carrier_pc.element.active = true;
    carrier_pc.pc.life_points = 100;
    carrier_pc.element.set_position(carrier_world);
    carrier_pc
        .element
        .set_position_map(MapPoint::new(321.25, 640.0));

    let exact_target_world = WorldPoint3D::new(12.345_679, 98.765_434, 7.654_321);
    let Entity::Pc(target_pc) = engine.ent_mut(target) else {
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

    let assets = engine.test_runtime_assets();

    let (ai_position, optical_point) = engine.enemy_optical_geometry_for_test(&assets, target);
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
    // A callback changes the carrier before its next movement synchronizes
    // the body. Effective AI position follows it; optical geometry stays on
    // the carried human's own stored world point.
    engine.place_map(carrier, MapPoint::new(999.0, 999.0));
    let (moved_ai, unmoved_optical) = engine.enemy_optical_geometry_for_test(&assets, target);
    assert_eq!((moved_ai.x, moved_ai.y), (999.0, 999.0));
    assert_eq!(unmoved_optical, expected_optical_point);
}
