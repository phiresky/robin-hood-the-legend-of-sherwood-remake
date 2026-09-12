use super::*;
use crate::element::{
    ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData, NpcData, PcData,
    SoldierData,
};
use crate::engine::movement::DoorPassAdvance;
use crate::sequence::{
    LegacyV48OrderState, LegacyV48SequenceElementState, SequenceElement, SequenceElementData,
    SequenceState,
};

fn make_soldier(sector: Option<u16>) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorSoldier;
        initial_element.active = true;
        initial_element
    };
    if let Some(sector) = sector {
        element.set_sector(crate::position_interface::SectorHandle::new(sector));
    }
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: SoldierData::default(),
    })
}

fn make_pc(sector: u16) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorPc;
        initial_element.active = true;
        initial_element
    };
    element.set_sector(crate::position_interface::SectorHandle::new(sector));
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

fn dispatch_pass(
    engine: &mut EngineInner,
    doors: &[crate::gate::Door],
    owner: EntityId,
) -> (PassDoorLaunchBarrier, crate::sequence::SequenceId) {
    let (posture_after_transition, action_state_after_transition) = engine
        .world
        .entities
        .get(owner)
        .map(|entity| {
            (
                entity.element_data().posture(),
                entity.actor_data().unwrap().action_state,
            )
        })
        .unwrap();
    dispatch_pass_with_transition_state(
        engine,
        doors,
        owner,
        OrderType::WalkingUpright,
        crate::sequence::MoveFlags::empty(),
        posture_after_transition,
        action_state_after_transition,
    )
}

fn dispatch_pass_with_transition_state(
    engine: &mut EngineInner,
    doors: &[crate::gate::Door],
    owner: EntityId,
    authored_action: OrderType,
    flags: crate::sequence::MoveFlags,
    posture_after_transition: Posture,
    action_state_after_transition: crate::element::ActionState,
) -> (PassDoorLaunchBarrier, crate::sequence::SequenceId) {
    dispatch_pass_with_element_mutation(
        engine,
        doors,
        owner,
        authored_action,
        flags,
        posture_after_transition,
        action_state_after_transition,
        |_| {},
    )
}

fn dispatch_pass_with_element_mutation(
    engine: &mut EngineInner,
    doors: &[crate::gate::Door],
    owner: EntityId,
    authored_action: OrderType,
    flags: crate::sequence::MoveFlags,
    posture_after_transition: Posture,
    action_state_after_transition: crate::element::ActionState,
    mutate_element: impl FnOnce(&mut SequenceElement),
) -> (PassDoorLaunchBarrier, crate::sequence::SequenceId) {
    for sector_number in doors
        .iter()
        .flat_map(|door| [door.sector_out, door.sector_in])
    {
        if engine
            .world
            .fast_grid
            .level
            .sector_number_map
            .contains_key(&sector_number)
        {
            continue;
        }
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        let index = level.sectors.len();
        level.sector_number_map.insert(sector_number, index);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number,
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
        });
    }
    let mut element = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(owner),
        authored_action,
    );
    element.posture_after_transition = posture_after_transition;
    element.action_state_after_transition = action_state_after_transition;
    let SequenceElementData::Movement {
        gate_id,
        flags: element_flags,
        ..
    } = &mut element.data
    else {
        unreachable!()
    };
    *gate_id = Some(crate::gate::DoorIndex::new(0).expect("valid door index"));
    *element_flags = flags;
    mutate_element(&mut element);
    let seq_id = engine.orders.sequence_manager.launch_element(element);
    let barrier = PassDoorLaunchContext::new(
        doors,
        &mut engine.world.entities,
        &engine.world.fast_grid,
        &mut engine.orders.sequence_manager,
        &mut engine.orders.next_order_id,
    )
    .dispatch(owner, seq_id, 0);
    (barrier, seq_id)
}

fn default_door() -> crate::gate::Door {
    crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(7),
        sector_in: crate::sector::SectorNumber::new(8),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(0),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(1),
        point_mid: MapPoint::new(20.0, 30.0),
        point_out: MapPoint::new(10.0, 30.0),
        point_in: MapPoint::new(30.0, 30.0),
        ..crate::gate::Door::default()
    }
}

#[test]
fn default_inside_outside_reserves_complete_translated_order_chain() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(8));
    let first_id = engine.orders.next_order_id;

    let (barrier, seq_id) = dispatch_pass(&mut engine, &[default_door()], owner);

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    assert_eq!(engine.orders.next_order_id, first_id + 4);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("PassDoor movement element remains installed");
    assert_eq!(element.current_order().unwrap().order_id.get(), first_id);
    let pass = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .unwrap();
    assert_eq!(
        pass.preallocated_order_ids
            .iter()
            .map(|id| id
                .expect("new translations reserve every order identity")
                .get())
            .collect::<Vec<_>>(),
        [first_id + 1, first_id + 2, first_id + 3]
    );
    assert!(matches!(pass.steps[0], DoorPassStep::PassingDoor));
    assert!(matches!(pass.steps[1], DoorPassStep::Walk { .. }));
    assert!(matches!(pass.steps[2], DoorPassStep::PassingDoor));
}

#[test]
fn pass_door_change_layer_and_sector_follows_pc_carried_actor() {
    for direct in [true, false] {
        let mut engine = EngineInner::new();
        let door = crate::gate::Door {
            layer_out: 2,
            layer_in: 5,
            ..default_door()
        };
        let (source_sector, source_layer, target_sector, target_layer, target_index) = if direct {
            (7, 2, 8, 5, crate::fast_find_grid::SectorIndex::new(1))
        } else {
            (8, 5, 7, 2, crate::fast_find_grid::SectorIndex::new(0))
        };
        let owner = engine.add_test_entity(make_pc(source_sector));
        let carried = engine.add_test_entity(make_soldier(Some(99)));
        {
            let owner_entity = engine.world.entities.get_mut(owner).unwrap();
            owner_entity.element_data_mut().set_layer(source_layer);
            owner_entity.pc_data_mut().unwrap().carried = Some(carried);
        }
        {
            let carried_entity = engine.world.entities.get_mut(carried).unwrap();
            carried_entity.set_posture(Posture::Carried);
            carried_entity.element_data_mut().set_layer(12);
            carried_entity.human_data_mut().unwrap().carrier = Some(owner);
        }

        let _ = dispatch_pass(&mut engine, std::slice::from_ref(&door), owner);
        engine.script_domains.interactables.doors.push(door);
        engine.execute_pass_door(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            direct,
            0,
        );

        for actor in [owner, carried] {
            let element = engine.world.entities.get(actor).unwrap().element_data();
            assert_eq!(element.layer(), target_layer);
            assert_eq!(element.sector().unwrap().get(), target_sector);
            assert_eq!(element.sector().unwrap().arena_index(), target_index);
        }
    }
}

#[test]
fn pass_door_change_layer_and_sector_does_not_rewrite_unrelated_actor() {
    let mut engine = EngineInner::new();
    let door = crate::gate::Door {
        layer_out: 2,
        layer_in: 5,
        ..default_door()
    };
    let owner = engine.add_test_entity(make_pc(7));
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_layer(2);
    let unrelated = engine.add_test_entity(make_soldier(Some(99)));
    engine
        .world
        .entities
        .get_mut(unrelated)
        .unwrap()
        .element_data_mut()
        .set_layer(12);

    let _ = dispatch_pass(&mut engine, std::slice::from_ref(&door), owner);
    engine.script_domains.interactables.doors.push(door);
    engine.execute_pass_door(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        true,
        0,
    );

    let owner_element = engine.world.entities.get(owner).unwrap().element_data();
    assert_eq!(owner_element.layer(), 5);
    assert_eq!(owner_element.sector().unwrap().get(), 8);
    let unrelated_element = engine.world.entities.get(unrelated).unwrap().element_data();
    assert_eq!(unrelated_element.layer(), 12);
    assert_eq!(unrelated_element.sector().unwrap().get(), 99);
}

fn install_lift_sector(engine: &mut EngineInner, lift_type: LiftType) {
    let lift_sector = crate::sector::SectorNumber::new(42);
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    if level.sectors.is_empty() {
        let outside_sector = crate::sector::SectorNumber::new(7);
        level.sector_number_map.insert(outside_sector, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number: outside_sector,
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
        });
    }
    let index = level.sectors.len();
    level.sector_number_map.insert(lift_sector, index);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::LIFT,
        layer: 0,
        sector_number: lift_sector,
        door_index: None,
        lift_type: Some(lift_type),
        lift_direction: 5,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
}

#[test]
fn fallback_wait_on_ladder_preserves_inherited_facing() {
    let mut engine = EngineInner::new();
    install_lift_sector(&mut engine, LiftType::Ladder);
    let owner = engine.add_test_entity(make_pc(42));
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.set_posture(Posture::OnLadder);
        entity.element_data_mut().set_direction_instantly(1);
    }

    engine.ensure_wait_element(owner);

    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(entity.element_data().direction(), 1);
    assert_eq!(entity.position_iface().get_direction_goal().as_u8(), 1);
    let element = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| {
            element.owner == Some(owner) && element.command == crate::element::Command::Wait
        })
        .expect("fallback Wait must be installed");
    assert_eq!(element.command, crate::element::Command::Wait);
}

#[test]
fn fallback_wait_preserves_unconscious_posture_inside_ladder_sector() {
    let mut engine = EngineInner::new();
    install_lift_sector(&mut engine, LiftType::Ladder);
    let owner = engine.add_test_entity(make_pc(42));
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.set_posture(Posture::Lying);
        entity.human_data_mut().unwrap().unconscious = true;
    }

    engine.ensure_wait_element(owner);

    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .posture(),
        Posture::Lying,
        "actor waiting must preserve live posture even when a fallen actor remains in a lift sector"
    );
}

fn bind_single_animation(engine: &mut EngineInner, owner: EntityId, action: OrderType) {
    let mut conversion = vec![
        crate::sprite_script::UNMAPPED;
        crate::sprite_script::NONANIMATION_END.max(action as usize + 1)
    ];
    conversion[action as usize] = 0;
    let played_action = match action {
        OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
        OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
        OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
        OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
        other => other,
    };
    if played_action as usize >= conversion.len() {
        conversion.resize(played_action as usize + 1, crate::sprite_script::UNMAPPED);
    }
    conversion[played_action as usize] = 0;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![10, 10, 10],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(std::iter::repeat_n(script, 16).collect()),
        std::sync::Arc::new(conversion),
    );
}

fn install_production_climb_fixture(
    engine: &mut EngineInner,
    owner: EntityId,
    lift_type: LiftType,
    action: OrderType,
) -> (crate::sequence::SequenceId, std::num::NonZeroU32) {
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .has_climb = true;
    install_lift_sector(engine, lift_type);
    let door = crate::gate::Door {
        door_type: DoorType::LiftLow,
        sector_in: crate::sector::SectorNumber::new(42),
        ..default_door()
    };
    engine.script_domains.interactables.doors.push(door.clone());
    let (_, seq_id) = dispatch_pass(engine, &[door], owner);
    bind_single_animation(engine, owner, action);

    let order_id = {
        let order = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, 0)
            .unwrap()
            .orders
            .front_mut()
            .unwrap();
        order.order_type = action;
        order.compute_direction = false;
        order.target_x = 20.0;
        order.target_y = 30.0;
        order.order_id
    };
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.element_data_mut().set_direction_instantly(0);
        entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(7));
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = crate::element::ActionState::Waiting;
        actor.active_door_pass.as_mut().unwrap().current_action = action;
    }
    engine.execute_pass_door(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        true,
        0,
    );
    (seq_id, order_id)
}

#[test]
fn launch_context_resolves_direction_and_installs_first_order_before_splice() {
    for (actor_sector, expected_direct, expected_exit) in [
        (7, true, MapPoint::new(30.0, 30.0)),
        (8, false, MapPoint::new(10.0, 30.0)),
    ] {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_soldier(Some(actor_sector)));
        let (barrier, seq_id) = dispatch_pass(&mut engine, &[default_door()], owner);

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        let order = element.current_order().expect("initial walk is installed");
        assert_eq!(order.order_type, OrderType::WalkingUpright);
        assert_eq!((order.target_x, order.target_y), (20.0, 30.0));
        let entity = engine.world.entities.get(owner).unwrap();
        assert!(!entity.position_iface().is_anti_collision_on());
        assert_eq!(
            entity.position_iface().get_door(),
            Some(crate::position_interface::DoorHandle::new(0).expect("valid door index")),
            "translated door movement must expose the sprite's exact door identity"
        );
        assert_eq!(
            entity.position_iface().get_door_direction(),
            expected_direct
        );
        let pass = entity
            .actor_data()
            .unwrap()
            .active_door_pass
            .as_ref()
            .expect("door pass is active before the splice");
        assert_eq!(pass.direct, expected_direct);
        let translated_exit = pass.steps.iter().find_map(|step| match step {
            DoorPassStep::Walk { destination, .. } => Some(*destination),
            _ => None,
        });
        assert_eq!(translated_exit, Some(expected_exit));
    }
}

#[test]
fn corpse_carrying_pass_rewrites_authored_fast_run_without_affecting_upright_control() {
    for (posture, expected_action) in [
        (Posture::CarryingCorpse, OrderType::WalkingWithCorpse),
        (Posture::Upright, OrderType::RunningUpright),
    ] {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_pc(7));
        engine
            .world
            .entities
            .get_mut(owner)
            .expect("door-pass test PC")
            .set_posture(posture);

        let (barrier, seq_id) = dispatch_pass_with_transition_state(
            &mut engine,
            &[default_door()],
            owner,
            OrderType::RunningUpright,
            crate::sequence::MoveFlags::FAST,
            posture,
            crate::element::ActionState::MovingFast,
        );

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("PassDoor element remains installed");
        let SequenceElementData::Movement { action, flags, .. } = &element.data else {
            unreachable!()
        };
        assert!(flags.contains(crate::sequence::MoveFlags::FAST));
        assert_eq!(*action, expected_action, "translated root action");
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(expected_action),
            "the selected first door rail must use the posture-adapted action"
        );

        let pass = engine
            .world
            .entities
            .get(owner)
            .expect("door-pass test PC remains live")
            .actor_data()
            .expect("door-pass test PC has actor data")
            .active_door_pass
            .as_ref()
            .expect("translated door pass remains active");
        assert_eq!(pass.current_action, expected_action);
        let remaining_walk_actions = pass
            .steps
            .iter()
            .filter_map(|step| match step {
                DoorPassStep::Walk { action, .. } => Some(*action),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!remaining_walk_actions.is_empty());
        assert!(
            remaining_walk_actions
                .iter()
                .all(|action| *action == expected_action),
            "every remaining translated door rail must retain {expected_action:?}: {remaining_walk_actions:?}"
        );
    }
}

#[test]
fn direct_pc_pass_uses_stamped_moving_fast_sword_state() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        crate::element::ActionState::Waiting,
        "the live state deliberately differs from the transition stamp"
    );
    let door = crate::gate::Door {
        door_type: DoorType::Building,
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass_with_transition_state(
        &mut engine,
        &[door],
        owner,
        OrderType::RunningUpright,
        crate::sequence::MoveFlags::FAST,
        Posture::Upright,
        crate::element::ActionState::MovingFastSword,
    );

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    let SequenceElementData::Movement { action, .. } = &element.data else {
        unreachable!()
    };
    assert_eq!(*action, OrderType::RunningWithSword);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::RunningWithSword),
        "the first direct-door rail must retain the stamped fast sword movement"
    );
}

#[test]
fn direct_pc_pass_preserves_authored_run_without_fast_flag_in_sword_state() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    let door = crate::gate::Door {
        door_type: DoorType::Building,
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass_with_transition_state(
        &mut engine,
        &[door],
        owner,
        OrderType::RunningUpright,
        crate::sequence::MoveFlags::empty(),
        Posture::Upright,
        crate::element::ActionState::MovingSword,
    );

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    let SequenceElementData::Movement { action, flags, .. } = &element.data else {
        unreachable!()
    };
    assert!(flags.is_empty());
    assert_eq!(*action, OrderType::RunningWithSword);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::RunningWithSword),
        "PassDoor must derive sword speed from its authored run, not its empty flags"
    );
}

#[test]
fn direct_stairs_pass_preserves_stamped_crouched_posture() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    install_lift_sector(&mut engine, LiftType::Stairs);
    let door = crate::gate::Door {
        door_type: DoorType::LiftHigh,
        sector_in: crate::sector::SectorNumber::new(42),
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass_with_transition_state(
        &mut engine,
        &[door],
        owner,
        OrderType::WalkingCrouched,
        crate::sequence::MoveFlags::empty(),
        Posture::Crouched,
        crate::element::ActionState::Moving,
    );

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    let SequenceElementData::Movement { action, .. } = &element.data else {
        unreachable!()
    };
    assert_eq!(*action, OrderType::WalkingCrouched);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::WalkingCrouched),
        "the walk to the stairs midpoint must retain the stamped crouched posture"
    );

    let pass = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .unwrap();
    assert_eq!(pass.current_action, OrderType::WalkingCrouched);
    assert!(pass.steps.iter().any(|step| {
        matches!(
            step,
            DoorPassStep::Walk {
                action: OrderType::WalkingCrouched,
                ..
            }
        )
    }));
}

#[test]
fn indirect_stairs_pass_translates_dormant_lying_stamp_from_live_lift() {
    let mut engine = EngineInner::new();
    install_lift_sector(&mut engine, LiftType::Stairs);
    let owner = engine.add_test_entity(make_pc(42));
    let door = crate::gate::Door {
        door_type: DoorType::LiftHigh,
        sector_in: crate::sector::SectorNumber::new(42),
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass_with_transition_state(
        &mut engine,
        &[door],
        owner,
        OrderType::WalkingUpright,
        crate::sequence::MoveFlags::empty(),
        Posture::Lying,
        crate::element::ActionState::Waiting,
    );

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("PassDoor element remains installed");
    let SequenceElementData::Movement { action, .. } = &element.data else {
        unreachable!()
    };
    assert_eq!(*action, OrderType::WalkingStairs);
    assert_eq!(
        element.current_order().map(|order| order.order_type),
        Some(OrderType::WalkingStairs),
        "the inside rail must use the live stairs sector despite the dormant lying stamp"
    );

    let pass = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .unwrap();
    let remaining_walk_actions = pass
        .steps
        .iter()
        .filter_map(|step| match step {
            DoorPassStep::Walk { action, .. } => Some(*action),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_walk_actions, [OrderType::WalkingStairs]);
}

#[test]
fn select_door_step_materializes_without_firing_its_hulk_callback() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    let actor = engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.active_door_pass = Some(ActiveDoorPass {
        door_index: crate::gate::DoorIndex::new(43).unwrap(),
        direct: true,
        position_direct: true,
        steps: [
            DoorPassStep::Select { speed: 1.25 },
            DoorPassStep::PassingDoor,
        ]
        .into(),
        preallocated_order_ids: [std::num::NonZeroU32::new(41), std::num::NonZeroU32::new(42)]
            .into(),
        triggers_fired: 0,
        current_action: OrderType::WalkingUpright,
        current_reverse: false,
        saved_action_state: None,
    });
    let next_order_id = engine.orders.next_order_id;
    let DoorPassAdvance::ActionPoint { order } = EngineInner::advance_door_pass(
        actor,
        owner,
        MapPoint::ZERO,
        &mut engine.orders.next_order_id,
    ) else {
        panic!("Select must materialize a real action-point order")
    };
    assert_eq!(order.order_type, OrderType::Select);
    assert_eq!(order.order_id.get(), 41);
    assert_eq!(order.tolerance, 1.25);
    assert_eq!(
        order.completion,
        crate::order::OrderCompletion::ResumeDoorPass
    );
    assert_eq!(engine.orders.next_order_id, next_order_id);
    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(entity.human_data().unwrap().running_hulk, 0);
    let pass = entity
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .unwrap();
    assert_eq!(pass.triggers_fired, 0);
    assert!(matches!(
        pass.steps.front(),
        Some(DoorPassStep::PassingDoor)
    ));
    assert_eq!(
        entity.element_data().sector(),
        crate::position_interface::SectorHandle::new(7)
    );

    // The callback consumed by the animation outcome lane still applies
    // the returned order's fade speed; materialization above did not run it.
    engine.apply_select_hulk(owner, order.tolerance);
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .running_hulk,
        25
    );
}

#[test]
fn wall_transition_and_passing_door_use_separate_owner_slots() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .has_climb = true;
    install_lift_sector(&mut engine, LiftType::Wall);
    let door = crate::gate::Door {
        door_type: DoorType::LiftLow,
        sector_in: crate::sector::SectorNumber::new(42),
        ..default_door()
    };
    engine.script_domains.interactables.doors.push(door.clone());
    let (_, seq_id) = dispatch_pass(&mut engine, &[door], owner);

    let transition_destination = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .element_data()
        .position_map();
    let transition = {
        let actor = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        EngineInner::advance_door_pass(
            actor,
            owner,
            transition_destination,
            &mut engine.orders.next_order_id,
        )
    };
    let DoorPassAdvance::Continue {
        order_id,
        destination,
        action,
        reverse,
        compute_direction,
        tolerance,
    } = transition
    else {
        panic!("low direct wall pass must schedule its climb transition: {transition:?}");
    };
    assert_eq!(action, OrderType::TransitionWaitingUprightClimbingWallUp);
    let mut transition_order =
        crate::order::Order::new(action, destination.x, destination.y, order_id);
    transition_order.reverse = reverse;
    transition_order.compute_direction = compute_direction;
    transition_order.tolerance = tolerance;
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_door_pass
            .as_ref()
            .unwrap()
            .triggers_fired,
        0,
        "materializing the transition cannot fire the following door action point"
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .sector(),
        crate::position_interface::SectorHandle::new(7)
    );

    engine
        .orders
        .sequence_manager
        .push_order_on(seq_id, 0, transition_order.clone());
    engine.do_next_order(seq_id, 0);

    bind_single_animation(
        &mut engine,
        owner,
        OrderType::TransitionWaitingUprightClimbingWallUp,
    );
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        // The preceding walk step has already delivered the actor to the
        // transition point; a terminated transition short of its goal
        // would instead spawn the Original's distance-continuation copy.
        entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(
                transition_order.target_x,
                transition_order.target_y,
            ));
        let sprite = &mut entity.element_data_mut().sprite;
        sprite
            .position_iface
            .set_sector(crate::position_interface::SectorHandle::new(7));
        sprite.last_processed_order_id = transition_order.order_id.get();
        sprite.last_action = OrderType::TransitionWaitingUprightClimbingWallUp;
        sprite.current_row = 0;
        sprite.current_frame = 2;
        sprite.frame_count = 9;
        sprite
            .position_iface
            .set_map_goal(crate::coordinates::MapPoint::new(
                transition_order.target_x,
                transition_order.target_y,
            ));
        sprite.position_iface.compute_increment_all(false);
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = crate::element::ActionState::Moving;
    }

    // The terminal transition slot applies its OnWall state and installs
    // PassingDoor, but does not execute the topology callback.
    let _ = engine.tick_entity_movement_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        Some(crate::engine::movement::MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id: transition_order.order_id,
        }),
    );

    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(
        entity.element_data().posture(),
        crate::element::Posture::OnWall
    );
    assert_eq!(
        entity.element_data().sector(),
        crate::position_interface::SectorHandle::new(7),
        "transition completion must leave topology on the source side"
    );
    let passing_order = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .current_order()
        .unwrap()
        .clone();
    assert_eq!(passing_order.order_type, OrderType::PassingDoor);
    let position_before = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .element_data()
        .position_map();

    let _ = engine.tick_entity_movement_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        Some(crate::engine::movement::MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id: passing_order.order_id,
        }),
    );

    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(
        entity.element_data().sector(),
        crate::position_interface::SectorHandle::new(42)
    );
    assert_eq!(entity.element_data().position_map(), position_before);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::ClimbingWallUp,
        "PassingDoor's slot may install, but must not execute, the climb successor"
    );
}

#[test]
fn far_side_projection_selection_does_not_change_door_topology() {
    let mut engine = EngineInner::new();
    std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level)
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(50), 0);
    let owner = engine.add_test_entity(make_pc(62));
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.element_data_mut().set_layer(3);
    }

    let midpoint = MapPoint::new(2276.0, 1136.0);
    engine.finalize_special_move_position_using_projection_sector(
        &LevelAssets::new(),
        owner,
        crate::engine::special_motion::SpecialMovePosition::Map(midpoint),
        2,
        50,
        MapPoint::new(2272.0, 1123.0),
        "test far-side door projection",
    );

    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(entity.element_data().position_map(), midpoint);
    assert_eq!(entity.element_data().layer(), 3);
    assert_eq!(
        entity.element_data().sector(),
        crate::position_interface::SectorHandle::new(62),
        "projection lookup is not the explicit PassingDoor topology swap"
    );
}

#[test]
fn direct_door_completion_preserves_rail_elevation_without_plane_reprojection() {
    let mut engine = EngineInner::new();
    engine
        .script_domains
        .interactables
        .doors
        .push(default_door());
    let owner = engine.add_test_entity(make_pc(7));

    let before_map = MapPoint::new(29.0, 30.0);
    let elevation = 93.3318_f32;
    let ground = crate::coordinates::GroundPoint::from_map_and_z(before_map, elevation);
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.position_iface_mut().set_obstacle(
            None,
            Some(crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 90.00101,
            }),
        );
        // The obstacle is already installed before the door-rail
        // movement writes its authoritative 3D endpoint.
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(
                ground.x, ground.y, elevation,
            ));
    }

    engine.commit_completed_door_pass_position(
        &LevelAssets::new(),
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        true,
    );

    let entity = engine.world.entities.get(owner).unwrap();
    assert_eq!(
        entity.element_data().position_map(),
        MapPoint::new(30.0, 30.0)
    );
    assert_eq!(
        entity.element_data().position().z.to_bits(),
        elevation.to_bits(),
        "the direct branch must preserve the door-rail Z instead of resolving the plane"
    );
    assert_eq!(
        entity.element_data().position().to_map(),
        entity.element_data().position_map(),
        "the endpoint snap must keep map and world coordinates coherent"
    );
}

#[test]
fn wall_up_transition_completion_recomputes_midpoint_on_installed_rail_plane() {
    use crate::sight_obstacle::{ObstaclePoint, SIGHTOBSTACLE_PROJECTION_AREA, SightObstacle};

    for start in [MapPoint::new(20.0, 30.0), MapPoint::new(10.0, 30.0)] {
        let mut engine = EngineInner::new();
        engine
            .script_domains
            .interactables
            .doors
            .push(default_door());
        let owner = engine.add_test_entity(make_pc(7));

        // Competing authored projection: resolving the source-sector
        // projection at the midpoint would flatten the actor to Z=90.
        // Original's transition arm does not perform that lookup.
        let mut flat_projection = SightObstacle::new(
            1,
            crate::sight_obstacle::SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_PROJECTION_AREA,
        );
        flat_projection.set_projection_area_ref(
            crate::position_interface::Layer::ZERO,
            crate::fast_find_grid::SectorIndex::new(7).unwrap(),
        );
        flat_projection.material = 2;
        flat_projection.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 110.0,
                z_bottom: 0.0,
                z_top: 90.0,
            },
            ObstaclePoint {
                x: 40.0,
                y: 110.0,
                z_bottom: 0.0,
                z_top: 90.0,
            },
            ObstaclePoint {
                x: 40.0,
                y: 130.0,
                z_bottom: 0.0,
                z_top: 90.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 130.0,
                z_bottom: 0.0,
                z_top: 90.0,
            },
        ];
        flat_projection.top_plane_points =
            [[0.0, 110.0, 90.0], [40.0, 110.0, 90.0], [0.0, 130.0, 90.0]];
        flat_projection.rebuild_geometry();
        let mut installed_rail = SightObstacle::new_default(2);
        installed_rail.material = 1;
        let mut assets = LevelAssets::new();
        assets.environment.static_sight_obstacles =
            std::sync::Arc::new(vec![flat_projection, installed_rail]);
        engine.world.static_sight_obstacle_active = vec![true, true];
        assert_eq!(
            engine.get_projection_area_index(
                &assets,
                crate::position_interface::SectorHandle::new(7)
                    .unwrap()
                    .with_arena_index(crate::fast_find_grid::SectorIndex::new(7).unwrap()),
                0,
                MapPoint::new(20.0, 30.0),
            ),
            crate::sight_obstacle::SightObstacleIndex::new(0),
            "the control projection must genuinely compete at the transition midpoint"
        );

        let rail_plane = crate::position_interface::PlaneZCoeffs {
            az: 0.1,
            bz: 0.0,
            dz: 91.3318,
        };
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            let pi = entity.position_iface_mut();
            pi.set_obstacle(
                crate::position_interface::ObstacleHandle::new(1),
                Some(rail_plane),
            );
            pi.set_material(crate::element::GameMaterial::Wood);
            pi.set_map_position(start);
            pi.set_old_map_position(start);
            pi.set_old_position(pi.get_position());
            entity.actor_data_mut().unwrap().active_door_pass = Some(ActiveDoorPass {
                door_index: crate::gate::DoorIndex::new(0).expect("valid door index"),
                direct: true,
                position_direct: true,
                steps: VecDeque::new(),
                preallocated_order_ids: Default::default(),
                triggers_fired: 0,
                // The concrete/lazy door queues can already have exposed
                // a copied successor by the time deferred completion
                // effects drain. The captured terminating action remains
                // authoritative.
                current_action: OrderType::WalkingUpright,
                current_reverse: false,
                saved_action_state: None,
            });
        }

        engine.apply_door_pass_transition_completion_side_effects(
            &assets,
            owner,
            OrderType::TransitionWaitingUprightClimbingWallUp,
        );

        let entity = engine.world.entities.get(owner).unwrap();
        let pi = entity.position_iface();
        assert_eq!(entity.element_data().posture(), Posture::OnWall);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            crate::element::ActionState::Moving,
            "the captured terminating transition, not the advanced live mirror, owns completion state"
        );
        assert_eq!(pi.map_position(), MapPoint::new(20.0, 30.0));
        assert_eq!(
            pi.get_obstacle(),
            crate::position_interface::ObstacleHandle::new(1)
        );
        assert_eq!(pi.get_plane(), Some(&rail_plane));
        assert_eq!(pi.get_material(), crate::element::GameMaterial::Wood);
        assert_eq!(
            pi.get_elevation().to_bits(),
            93.3318_f32.to_bits(),
            "map assignment plus full position recomputation must project the midpoint on the installed rail plane"
        );
        if start == MapPoint::new(20.0, 30.0) {
            assert!(
                !pi.is_moving(),
                "an in-place transition completion must not publish phantom 3D movement"
            );
        }
    }
}

#[test]
fn restored_ladder_pass_uses_serialized_live_door_for_transition_completion() {
    use crate::element::ActionState;
    use crate::position_interface::DoorHandle;

    for (action, initial_posture, initial_state, expected_posture, expected_state) in [
        (
            OrderType::TransitionWaitingCrouchedClimbingLadderDown,
            Posture::Crouched,
            ActionState::Waiting,
            Posture::OnLadder,
            ActionState::Moving,
        ),
        (
            OrderType::TransitionClimbingLadderDownWaitingUprightAlerted,
            Posture::OnLadder,
            ActionState::Moving,
            Posture::Upright,
            ActionState::Waiting,
        ),
    ] {
        let mut engine = EngineInner::new();
        engine
            .script_domains
            .interactables
            .doors
            .push(default_door());
        let owner = engine.add_test_entity(make_pc(7));
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.set_posture(initial_posture);
            entity.actor_data_mut().unwrap().action_state = initial_state;
            entity
                .position_iface_mut()
                .set_door(DoorHandle::new(0).expect("valid door index"), true);
            assert!(entity.actor_data().unwrap().active_door_pass.is_none());
        }

        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            action,
        );

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().posture(), expected_posture);
        assert_eq!(entity.actor_data().unwrap().action_state, expected_state);
        if action == OrderType::TransitionWaitingCrouchedClimbingLadderDown {
            assert_eq!(
                entity.element_data().position_map(),
                MapPoint::new(30.0, 30.0),
                "ladder-entry completion snaps to the serialized door's inside point"
            );
        }
    }
}

#[test]
fn direct_door_completion_does_not_reconstruct_an_already_committed_endpoint() {
    let mut engine = EngineInner::new();
    let endpoint = MapPoint::new(250.0, 270.0);
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: endpoint,
            ..default_door()
        });
    let owner = engine.add_test_entity(make_pc(7));

    // At this magnitude, `(map_y + z) - z` is one ULP below map_y.
    // That is exactly why the original game's direct door passage leaves the final rail
    // position alone instead of recomputing it.
    let elevation = 244.555_7_f32;
    let ground = crate::coordinates::GroundPoint::from_map_and_z(endpoint, elevation);
    let world = crate::coordinates::WorldPoint3D::new(ground.x, ground.y, elevation);
    assert_ne!(world.to_map().y.to_bits(), endpoint.y.to_bits());
    {
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity.element_data_mut().set_position(world);
        // Preserve the independently committed original-game world
        // representation; map XY is authoritative for movement parity.
        entity
            .element_data_mut()
            .set_position_map_preserving_3d(endpoint);
    }
    let before = engine.get_entity(owner).unwrap().element_data().position();

    engine.commit_completed_door_pass_position(
        &LevelAssets::new(),
        owner,
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        true,
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.element_data().position_map(), endpoint);
    assert_eq!(entity.element_data().position(), before);
}

#[test]
fn denied_door_disables_anti_collision_before_marking_impossible() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    let door = crate::gate::Door {
        locked_pc: true,
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

    assert_eq!(barrier, PassDoorLaunchBarrier::SkipSplice);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .orders
            .is_empty()
    );
    assert!(
        !engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .position_iface()
            .is_anti_collision_on(),
        "door traversal disables anti-collision before authorization"
    );
}

#[test]
fn wall_lift_rejects_soldier_before_installing_an_order() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(7)));
    let lift_sector = crate::sector::SectorNumber::new(42);
    install_lift_sector(&mut engine, LiftType::Wall);
    let door = crate::gate::Door {
        door_type: DoorType::LiftHigh,
        sector_in: lift_sector,
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

    assert_eq!(barrier, PassDoorLaunchBarrier::SkipSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::Impossible);
    assert!(element.orders.is_empty());
    assert!(
        !engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .position_iface()
            .is_anti_collision_on()
    );
}

#[test]
fn ladder_lift_uses_ladder_translation_and_reaches_splice() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(7)));
    let lift_sector = crate::sector::SectorNumber::new(42);
    install_lift_sector(&mut engine, LiftType::Ladder);
    let door = crate::gate::Door {
        door_type: DoorType::LiftHigh,
        sector_in: lift_sector,
        ..default_door()
    };

    let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    let initial_walk = element.current_order().unwrap();
    assert_eq!(initial_walk.order_type, OrderType::WalkingUpright);
    assert!(initial_walk.reverse);
    let pass = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .unwrap();
    assert!(matches!(
        pass.steps.front(),
        Some(DoorPassStep::Transition {
            action: OrderType::Turning,
            reverse: true,
        })
    ));
}

#[test]
fn building_trap_exact_target_decorative_ladder_uses_release_compatibility_state() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(7)));
    let door = crate::gate::Door {
        door_type: DoorType::BuildingTrap,
        ..default_door()
    };
    engine.script_domains.interactables.doors.push(door.clone());

    let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);
    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_door_pass
            .as_ref()
            .unwrap()
            .steps
            .iter()
            .any(|step| matches!(
                step,
                DoorPassStep::Walk {
                    destination,
                    action: OrderType::ClimbingLadderDown,
                    reverse: true,
                    ..
                } if *destination == MapPoint::new(30.0, 30.0)
            ))
    );

    bind_single_animation(&mut engine, owner, OrderType::ClimbingLadderDown);
    let order_id = engine.orders.allocate_order_id();
    let passing_id = engine.orders.allocate_order_id();
    {
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, 0)
            .unwrap();
        element.orders.clear();
        let mut order =
            crate::order::Order::new(OrderType::ClimbingLadderDown, 30.0, 30.0, order_id);
        order.reverse = true;
        order.compute_direction = false;
        element.orders.push_back(order);
        element.orders.push_back(crate::order::Order::new(
            OrderType::PassingDoor,
            0.0,
            0.0,
            passing_id,
        ));
    }
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(30.0, 30.0));
        entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(8));
        entity.element_data_mut().set_direction_instantly(1);
        entity.element_data_mut().set_direction_goal(1);
        let actor = entity.actor_data_mut().unwrap();
        actor.execute_order_initialising = true;
        actor.active_door_pass.as_mut().unwrap().current_action = OrderType::ClimbingLadderDown;
        actor.active_door_pass.as_mut().unwrap().current_reverse = true;
    }

    let _ = engine.tick_entity_movement_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        Some(crate::engine::movement::MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id,
        }),
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(entity.element_data().direction(), 0);
    assert_eq!(entity.position_iface().get_direction_goal().as_u8(), 0);
    assert_eq!(entity.element_data().sprite.current_row, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_id,
        passing_id,
        "the exact-target decorative row must retire into the authored PassingDoor tail"
    );
}

#[test]
fn real_ladder_nonzero_climb_keeps_lift_facing_and_posture() {
    let mut engine = EngineInner::new();
    install_lift_sector(&mut engine, LiftType::Ladder);
    let owner = engine.add_test_entity(make_soldier(Some(42)));
    let door = crate::gate::Door {
        door_type: DoorType::LiftLow,
        sector_in: crate::sector::SectorNumber::new(42),
        ..default_door()
    };
    engine.script_domains.interactables.doors.push(door.clone());
    let (_, seq_id) = dispatch_pass(&mut engine, &[door], owner);
    bind_single_animation(&mut engine, owner, OrderType::ClimbingLadderDown);
    let order_id = engine.orders.allocate_order_id();
    {
        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, 0)
            .unwrap();
        element.orders.clear();
        let mut order =
            crate::order::Order::new(OrderType::ClimbingLadderDown, 40.0, 30.0, order_id);
        order.reverse = true;
        order.compute_direction = false;
        element.orders.push_back(order);
    }
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(30.0, 30.0));
        entity
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(42));
        entity.element_data_mut().set_direction_instantly(1);
        entity.element_data_mut().set_direction_goal(1);
        let actor = entity.actor_data_mut().unwrap();
        actor.execute_order_initialising = true;
        actor.active_door_pass.as_mut().unwrap().current_action = OrderType::ClimbingLadderDown;
        actor.active_door_pass.as_mut().unwrap().current_reverse = true;
    }

    let _ = engine.tick_entity_movement_owner(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        Some(crate::engine::movement::MovementOwnerSelection {
            seq_id,
            elem_idx: 0,
            order_id,
        }),
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.element_data().posture(), Posture::OnLadder);
    assert_eq!(entity.position_iface().get_direction_goal().as_u8(), 5);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_id,
        order_id,
        "a nonzero real ladder row must remain live after its Start edge"
    );
}

#[test]
#[should_panic(expected = "has no sector for door 0 direction resolution")]
fn missing_actor_sector_is_an_invariant_failure() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(None));

    let _ = dispatch_pass(&mut engine, &[default_door()], owner);
}

/// Actor translation tests only whether the current sector is the inside sector
/// for all door types; the
/// exit-sector validity check guarding the other branch is
/// compiled out of the shipped build. An actor standing in a third
/// sector therefore passes the door directly.
#[test]
fn actor_sector_outside_both_door_sides_passes_directly() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(99)));

    let (barrier, _seq_id) = dispatch_pass(&mut engine, &[default_door()], owner);

    assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
    let entity = engine.world.entities.get(owner).unwrap();
    assert!(entity.position_iface().get_door_direction());
    let pass = entity
        .actor_data()
        .unwrap()
        .active_door_pass
        .as_ref()
        .expect("door pass is active before the splice");
    assert!(pass.direct);
}

#[test]
#[should_panic(expected = "is a lift door but sector 8 has no lift type")]
fn lift_door_requires_canonical_lift_type() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(7)));
    let door = crate::gate::Door {
        door_type: DoorType::LiftHigh,
        ..default_door()
    };

    let _ = dispatch_pass(&mut engine, &[door], owner);
}

#[test]
fn production_lift_callbacks_and_transition_turn_without_snapping_in_swapped_creation_order() {
    for (lift_type, action, expected_direction) in [
        (
            LiftType::Ladder,
            OrderType::TransitionWaitingUprightClimbingLadderUp,
            1,
        ),
        (LiftType::Ladder, OrderType::ClimbingLadderUpFast, 2),
        (
            LiftType::Wall,
            OrderType::TransitionWaitingUprightClimbingWallUp,
            1,
        ),
        (LiftType::Wall, OrderType::ClimbingWallUpFast, 2),
    ] {
        for owner_is_earlier in [true, false] {
            let mut engine = EngineInner::new();
            let owner = if owner_is_earlier {
                let owner = engine.add_test_entity(make_pc(7));
                let _observer = engine.add_test_entity(make_soldier(Some(7)));
                owner
            } else {
                let _observer = engine.add_test_entity(make_soldier(Some(7)));
                engine.add_test_entity(make_pc(7))
            };
            engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .pc_data_mut()
                .unwrap()
                .has_climb = true;
            install_lift_sector(&mut engine, lift_type);
            let door = crate::gate::Door {
                door_type: DoorType::LiftLow,
                sector_in: crate::sector::SectorNumber::new(42),
                ..default_door()
            };
            engine.script_domains.interactables.doors.push(door.clone());
            let (_, seq_id) = dispatch_pass(&mut engine, &[door], owner);
            bind_single_animation(&mut engine, owner, action);

            let elem_idx = 0;
            let order_id = {
                let order = engine
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .unwrap()
                    .orders
                    .front_mut()
                    .unwrap();
                order.order_type = action;
                order.compute_direction = false;
                order.target_x = 20.0;
                order.target_y = 30.0;
                order.order_id
            };
            {
                let entity = engine.world.entities.get_mut(owner).unwrap();
                entity.element_data_mut().set_direction_instantly(i16::from(
                    crate::position_interface::Direction::NORTH,
                ));
                entity
                    .element_data_mut()
                    .set_sector(crate::position_interface::SectorHandle::new(7));
                let actor = entity.actor_data_mut().unwrap();
                actor.action_state = crate::element::ActionState::Waiting;
                actor.execute_order_initialising = true;
                actor.active_door_pass.as_mut().unwrap().current_action = action;
            }

            engine.execute_pass_door(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                crate::gate::DoorIndex::new(0).expect("valid door index"),
                true,
                0,
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap()
                    .element_data()
                    .direction(),
                0,
                "the PassingDoor action point changes sector but must not snap lift facing"
            );

            let _ = engine.tick_entity_movement_owner(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                Some(crate::engine::movement::MovementOwnerSelection {
                    seq_id,
                    elem_idx,
                    order_id,
                }),
            );

            let entity = engine.world.entities.get(owner).unwrap();
            assert_eq!(
                entity.element_data().direction(),
                expected_direction,
                "fast climb Execute must run its two original Turn() iterations"
            );
            assert_eq!(
                entity.position_iface().get_direction_goal().as_u8(),
                5,
                "{lift_type:?} transition must use the inside lift sector's direction goal"
            );
            engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .element_data_mut()
                .set_direction_goal(7);
            engine.apply_completed_door_pass_lift_entry_state(
                owner,
                crate::gate::DoorIndex::new(0).expect("valid door index"),
                true,
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap()
                    .element_data()
                    .direction(),
                expected_direction,
                "door-pass completion must preserve the gradual Turn() result"
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal()
                    .as_u8(),
                7,
                "PassingDoor changes lift topology but must not set a new facing goal"
            );
        }
    }
}

#[test]
fn frozen_all_climbs_turn_in_owner_slot_with_real_swapped_owner_visibility() {
    for (action, expected_direction) in [
        (OrderType::TransitionWaitingUprightClimbingLadderUp, 1),
        (OrderType::ClimbingLadderUpFast, 2),
    ] {
        for climber_is_earlier in [true, false] {
            let mut engine = EngineInner::new();
            let (climber, observer) = if climber_is_earlier {
                (
                    engine.add_test_entity(make_pc(7)),
                    engine.add_test_entity(make_pc(7)),
                )
            } else {
                let observer = engine.add_test_entity(make_pc(7));
                let climber = engine.add_test_entity(make_pc(7));
                (climber, observer)
            };
            let _ =
                install_production_climb_fixture(&mut engine, climber, LiftType::Ladder, action);
            engine.set_actors_frozen(true);

            let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
            let mut direction_seen_by_observer = None;
            engine.tick_actor_owner_envelopes_with_test_owner_hook(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                &positions,
                |engine, completed_owner| {
                    if completed_owner == observer {
                        direction_seen_by_observer = Some(
                            engine
                                .world
                                .entities
                                .get(climber)
                                .unwrap()
                                .element_data()
                                .direction(),
                        );
                    }
                },
            );

            assert_eq!(
                engine
                    .world
                    .entities
                    .get(climber)
                    .unwrap()
                    .element_data()
                    .direction(),
                expected_direction,
                "FrozenAll suppresses sprite motion, not climb Execute Turn()"
            );
            assert_eq!(
                direction_seen_by_observer,
                Some(if climber_is_earlier {
                    expected_direction
                } else {
                    0
                }),
                "only the genuinely later owner slot may see the frozen climb turn"
            );
        }
    }
}

#[test]
fn fast_climb_first_iteration_termination_prevents_second_turn_in_swapped_creation_order() {
    for owner_is_earlier in [true, false] {
        let mut engine = EngineInner::new();
        let owner = if owner_is_earlier {
            let owner = engine.add_test_entity(make_pc(7));
            let _observer = engine.add_test_entity(make_pc(7));
            owner
        } else {
            let _observer = engine.add_test_entity(make_pc(7));
            engine.add_test_entity(make_pc(7))
        };
        let (seq_id, order_id) = install_production_climb_fixture(
            &mut engine,
            owner,
            LiftType::Wall,
            OrderType::ClimbingWallUpFast,
        );
        {
            let element = engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .element_data_mut();
            // Walk/Run motion terminates only on map arrival, never on
            // the animation loop. Give the climb frames real distance and
            // start within one projected step of the goal so the first
            // motion-step pair reaches it.
            let mut conversion = vec![
                crate::sprite_script::UNMAPPED;
                crate::sprite_script::NONANIMATION_END
                    .max(OrderType::ClimbingWallUpFast as usize + 1)
            ];
            conversion[OrderType::ClimbingWallUpFast as usize] = 0;
            conversion[OrderType::ClimbingWallUp as usize] = 0;
            let script = crate::sprite_script::SpriteScript {
                action_id: OrderType::ClimbingWallUpFast as u16,
                action_done: 1,
                average_speed: 10.0,
                hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
                sum_distance: 30,
                frame_ids: vec![1, 2, 3],
                delays: vec![10, 10, 10],
                distances: vec![10, 10, 10],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                sound_ids: vec![0, 0, 0],
            };
            element.sprite = crate::sprite::Sprite::new(
                std::sync::Arc::new(std::iter::repeat_n(script, 16).collect()),
                std::sync::Arc::new(conversion),
            );
            element.set_position_map(crate::coordinates::MapPoint::new(20.0, 26.0));
            let sprite = &mut element.sprite;
            sprite
                .position_iface
                .set_sector(crate::position_interface::SectorHandle::new(7));
            // Door-pass translation disables anti-collision for the
            // duration of the pass.
            sprite.position_iface.set_anti_collision_on(false);
            sprite.last_processed_order_id = order_id.get();
            sprite.last_action = OrderType::ClimbingWallUp;
            sprite.current_row = 0;
            sprite.current_frame = 2;
            sprite.frame_count = 10;
            sprite
                .position_iface
                .set_map_goal(crate::coordinates::MapPoint::new(20.0, 30.0));
            sprite.position_iface.compute_increment_all(false);
        }
        // The skipped initialising Execute stamped the lift direction as
        // the retained turn goal; only the per-tick Turn steps remain.
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_goal(5);

        let _ = engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(crate::engine::movement::MovementOwnerSelection {
                seq_id,
                elem_idx: 0,
                order_id,
            }),
        );

        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .element_data()
                .direction(),
            1,
            "first fast-motion termination must skip the second turn-and-motion pair"
        );
    }
}

#[test]
#[should_panic(expected = "is not movement data")]
fn non_movement_pass_door_is_an_invariant_failure() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_soldier(Some(7)));
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            crate::element::Command::PassDoor,
            Some(owner),
        ));

    PassDoorLaunchContext::new(
        &[default_door()],
        &mut engine.world.entities,
        &engine.world.fast_grid,
        &mut engine.orders.sequence_manager,
        &mut engine.orders.next_order_id,
    )
    .dispatch(owner, seq_id, 0);
}

/// Actor translation only assigns the direct-door-passing flag inside
/// the original game's building, ladder, wall, and stairs translation helpers.
/// The `DOOR_REINFORCEMENT / DOOR_DEFAULT / DOOR_GATE / DOOR_TRAP` arm
/// inlines its own order chain and leaves
/// the latch alone, so AI destination forecasting keeps reading the value
/// the actor's last building or lift pass wrote.
#[test]
fn only_building_and_lift_passes_write_passing_door_directly() {
    for (door_type, expect_latch_written) in [
        (DoorType::Building, true),
        (DoorType::BuildingTrap, true),
        (DoorType::Default, false),
        (DoorType::Gate, false),
        (DoorType::Trap, false),
    ] {
        let mut engine = EngineInner::new();
        // Enter through `sector_out`, which is the `direct == true` side.
        let owner = engine.add_test_entity(make_pc(7));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .passing_door_directly = false;
        let door = crate::gate::Door {
            door_type,
            ..default_door()
        };

        dispatch_pass(&mut engine, &[door], owner);

        let actor = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap();
        assert_eq!(
            actor.active_door_pass.as_ref().map(|pass| pass.direct),
            Some(true),
            "{door_type:?} must still record a direct traversal"
        );
        assert_eq!(
            actor.passing_door_directly, expect_latch_written,
            "{door_type:?} wrote mbPassingDoorDirectly = {}",
            actor.passing_door_directly
        );
    }
}

fn loaded_v48_pass_state(order_state: Vec<LegacyV48OrderState>) -> LegacyV48SequenceElementState {
    LegacyV48SequenceElementState {
        deleted: false,
        script_driven: false,
        raw_dormant_posture_after_transition: None,
        raw_dormant_action_state_after_transition: None,
        next: None,
        postponed: None,
        mummy: None,
        linked_seek: Some(None),
        damage_arrow: None,
        raw_sword_strike: None,
        raw_dormant_movement_action: None,
        order_state,
        generic_raw_unions: Vec::new(),
    }
}

#[test]
fn untranslated_loaded_pass_door_ignores_dormant_saved_direction() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    let action_state = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .action_state;
    let door = crate::gate::Door {
        door_type: DoorType::Building,
        ..default_door()
    };

    dispatch_pass_with_element_mutation(
        &mut engine,
        &[door],
        owner,
        OrderType::WalkingUpright,
        crate::sequence::MoveFlags::empty(),
        Posture::Upright,
        action_state,
        |element| {
            let SequenceElementData::Movement { direction, .. } = &mut element.data else {
                unreachable!()
            };
            *direction = 0;
            element.legacy_v48 = Some(loaded_v48_pass_state(Vec::new()));
        },
    );

    let actor = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(
        actor
            .active_door_pass
            .as_ref()
            .map(|pass| pass.position_direct),
        Some(true)
    );
    assert!(actor.passing_door_directly);
}

#[test]
fn translated_loaded_pass_door_retains_saved_direction() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_pc(7));
    let action_state = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .action_state;
    let door = crate::gate::Door {
        door_type: DoorType::Building,
        ..default_door()
    };

    dispatch_pass_with_element_mutation(
        &mut engine,
        &[door],
        owner,
        OrderType::WalkingUpright,
        crate::sequence::MoveFlags::empty(),
        Posture::Upright,
        action_state,
        |element| {
            let SequenceElementData::Movement { direction, .. } = &mut element.data else {
                unreachable!()
            };
            *direction = 0;
            element.legacy_v48 = Some(loaded_v48_pass_state(vec![LegacyV48OrderState {
                legacy_id: 1,
            }]));
        },
    );

    let actor = engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .actor_data()
        .unwrap();
    assert_eq!(
        actor
            .active_door_pass
            .as_ref()
            .map(|pass| pass.position_direct),
        Some(false)
    );
    assert!(!actor.passing_door_directly);
}
