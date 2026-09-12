use super::*;

#[test]
fn fused_owner_gates_keep_fried_frozen_and_inactive_original_boundaries() {
    use super::super::tick::{ActorOwnerEnvelopePhase as Phase, capture_actor_owner_envelope};

    let mut engine = EngineInner::new();
    let inactive = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));
    let fried = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let npc = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Pc(inactive_pc) = engine.get_entity_mut(inactive).expect("inactive PC exists")
    else {
        panic!("inactive PC changed kind")
    };
    inactive_pc.element.active = false;
    inactive_pc
        .element
        .set_position_map(MapPoint::new(77.0, 88.0));
    inactive_pc.human.tiredness = 100;
    let Entity::Pc(fried_pc) = engine.get_entity_mut(fried).expect("fried PC exists") else {
        panic!("fried PC changed kind")
    };
    fried_pc.pc.fried_psykokwack = true;
    fried_pc.actor.produced_noise = None;
    engine.set_actors_frozen(true);
    engine.control.frame_counter = engine.world.original_creation_order(inactive) & 31;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("inactive PC fixture has a character profile")
        .endurance = 100;
    let positions = engine.boundary_positions_snapshot();

    let (_, trace) = capture_actor_owner_envelope(|| {
        crate::sim_rng::with_seed(0xA013_6A7E, |sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    assert!(
        !trace.iter().any(|phase| match phase {
            Phase::SoldierPrelude(owner)
            | Phase::Patrol(owner)
            | Phase::HumanPrelude(owner)
            | Phase::BaseActor(owner)
            | Phase::MovementExecute(owner)
            | Phase::HumanNoise(owner)
            | Phase::HumanTiredness(owner)
            | Phase::PcTail(owner)
            | Phase::NpcTail(owner) => *owner == fried,
        }),
        "fried PCs return before Human, Actor, noise, tiredness, and healing"
    );
    assert!(trace.contains(&Phase::BaseActor(npc)));
    assert!(trace.contains(&Phase::NpcTail(npc)));
    assert!(!trace.contains(&Phase::Patrol(npc)));
    let inactive_pc = engine
        .get_entity(inactive)
        .and_then(Entity::as_pc)
        .expect("inactive PC remains installed");
    let noise = inactive_pc
        .actor
        .produced_noise
        .expect("inactive PC still refreshes noise metadata");
    assert_eq!((noise.origin.x, noise.origin.y), (77.0, 88.0));
    assert_eq!(noise.volume, 0, "only inactivity/building zeroes PC noise");
    assert!(
        inactive_pc.human.tiredness < 100,
        "inactive/dead humans still recover tiredness on their staggered slot"
    );
    assert!(
        engine
            .get_entity(fried)
            .and_then(Entity::actor_data)
            .expect("fried PC remains an actor")
            .produced_noise
            .is_none(),
        "fried return must precede produced-noise refresh"
    );
}

#[test]
fn patrol_refresh_uses_owner_relative_member_positions_and_spawn_fallback() {
    use crate::ai::AiState;
    use crate::element::{Camp, Entity};

    fn member_is_admitted(member_before_chief: bool, spawn_after_snapshot: bool) -> bool {
        let mut engine = EngineInner::new();
        let (chief, initial_member) = if member_before_chief {
            let member = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            (chief, Some(member))
        } else {
            let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let member = (!spawn_after_snapshot)
                .then(|| engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists)));
            (chief, member)
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = engine.boundary_positions_snapshot();
        let member = initial_member
            .unwrap_or_else(|| engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists)));

        for id in [chief, member] {
            let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
                unreachable!()
            };
            soldier.element.active = true;
            soldier.npc.life_points = 100;
            soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
            soldier.npc.ai_brain.base_mut().unwrap().current_state = AiState::Default;
        }
        engine
            .get_entity_mut(chief)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        let member_before = MapPoint::new(50.0, 0.0);
        let member_after = if member_before_chief {
            MapPoint::new(500.0, 0.0)
        } else {
            MapPoint::new(50.0, 0.0)
        };
        if positions.get(member).is_some() {
            positions[member] = Some(crate::entities::BoundaryPosition {
                map: member_before,
                world: crate::coordinates::WorldPoint3D::new(member_before.x, member_before.y, 0.0),
            });
        }
        engine
            .get_entity_mut(member)
            .unwrap()
            .element_data_mut()
            .set_position_map(member_after);
        let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
            unreachable!()
        };
        chief_entity.npc.view_radius = 100;
        let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
        chief_ai.needs_patrol_reinit = true;
        chief_ai.theoretical_patrol = vec![member];

        crate::sim_rng::with_seed(0x0A01_3705, |sim| {
            engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
        });
        engine
            .get_entity(chief)
            .unwrap()
            .ai_controller()
            .unwrap()
            .patrol
            .contains(&member)
    }

    assert!(
        member_is_admitted(false, false),
        "an earlier chief must see a later member at its preserved pre-movement position"
    );
    assert!(
        !member_is_admitted(true, false),
        "a later chief must see an earlier member at its already-completed post-movement position"
    );
    assert!(
        member_is_admitted(false, true),
        "a callback-spawned later member absent from the oracle must use its current, never-moved position"
    );
}

#[test]
fn locked_owner_stops_at_gate_without_blocking_later_unlocked_owner() {
    use super::super::ai::{
        NpcPostDetectionTailPhase as Tail, capture_npc_post_detection_tail_phases,
    };
    use crate::ai::AiLockFlags;

    let mut engine = EngineInner::new();
    let locked = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unlocked = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [locked, unlocked] {
        let entity = engine.get_entity_mut(id).expect("gate owner exists");
        entity.element_data_mut().active = true;
        for list in &mut entity
            .npc_data_mut()
            .expect("gate owner has NPC data")
            .detectable_lists
        {
            list.clear();
        }
    }
    let ai = engine
        .get_entity_mut(locked)
        .and_then(Entity::ai_controller_mut)
        .expect("locked owner has AI");
    ai.locks_flag_field = AiLockFlags::FREEZE;
    ai.when_does_timer_ring = u32::MAX;
    ai.when_does_macro_timer_ring = u32::MAX;
    ai.emoticon_expiration_date = u32::MAX;

    let positions = engine.boundary_positions_snapshot();
    let (_, trace) = capture_npc_post_detection_tail_phases(|| {
        crate::sim_rng::with_seed(0xA013_10CC, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        })
    });
    let locked_trace: Vec<_> = trace
        .iter()
        .filter_map(|(id, phase)| (*id == locked).then_some(*phase))
        .collect();
    assert_eq!(
        locked_trace,
        vec![
            Tail::Ambush,
            Tail::Deafness,
            Tail::Busy,
            Tail::Ladder,
            Tail::RandomSpeech,
            Tail::LockGate,
        ]
    );
    assert_eq!(
        trace.last(),
        Some(&(unlocked, Tail::QueuedStimuli)),
        "the later owner must execute its whole unlocked tail"
    );
    let ai = engine
        .get_entity(locked)
        .and_then(Entity::ai_controller)
        .expect("locked owner retains AI");
    assert_eq!(ai.when_does_timer_ring, 0);
    assert_eq!(ai.when_does_macro_timer_ring, 0);
    assert_eq!(ai.emoticon_expiration_date, 0);
}

#[test]
fn sampled_open_gate_does_not_recheck_lock_or_global_freeze_inside_suffix() {
    use crate::ai::{AiLockFlags, EmoticonType, StimulusType, Substate};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 100;

    assert!(
        !engine.tick_npc_lock_gate_for_npc(npc_id),
        "the pre-suffix gate is sampled open"
    );

    // Model a synchronous periodic-update/EVENT_TIMER consequence that acquires
    // both kinds of outer lock after the branch has already been entered.
    engine.set_actors_frozen(true);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .expect("post-gate owner has AI");
    ai.locks_flag_field = AiLockFlags::FREEZE;
    ai.timer_is_running = true;
    ai.when_does_timer_ring = 100;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 100;
    ai.current_substate = Substate::DefaultOnPost;
    ai.set_transient_emoticon(EmoticonType::QuestionMark, 1, 99);

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);
    engine.tick_ai_macro_timer_for_npc(sim, npc_id, &assets);
    engine.tick_npc_emoticon_expiration_for_npc(npc_id);
    engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets);

    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("post-gate owner retains AI");
    assert!(!ai.timer_is_running, "due normal timer is consumed");
    assert!(!ai.macro_timer_is_running, "due macro timer is consumed");
    assert_eq!(ai.current_emoticon_type, EmoticonType::None);
    assert!(
        ai.stimulus_queue
            .iter()
            .any(|stimulus| stimulus.stimulus_type == StimulusType::EventTimer),
        "Think observes the new AI lock and the retained loop preserves it"
    );
}

#[test]
fn owner_tail_and_empty_common_drain_do_not_draw_unrelated_building_exit_gate() {
    use crate::ai::AmbushPoint;
    use crate::element::ActiveDoorPass;
    use crate::fast_find_grid::GridSector;
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};
    use crate::sector::{SectorNumber, SectorType};
    use crate::sim_rng::{RngSite, with_draw_trace};
    use std::collections::VecDeque;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let quiet_owner = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let door_actor = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(quiet_owner)
        .expect("quiet civilian exists")
    else {
        panic!("quiet owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("quiet civilian has AI");
    ai.current_state = crate::ai::AiState::Default;
    ai.current_substate = crate::ai::Substate::DefaultInMacro;
    ai.timer_is_running = false;
    ai.macro_command = vec![3, 8, 0]; // CMD_FACE_TO(8)
    ai.macro_command_offset = 0;
    ai.number_of_remaining_macro_bytes = 3;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;
    ai.stimulus_queue.push(crate::ai::Stimulus::new(
        crate::ai::StimulusType::EventOutOfView,
    ));

    let Entity::Pc(pc) = engine
        .get_entity_mut(door_actor)
        .expect("door-passing actor exists")
    else {
        panic!("door-passing actor changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.actor.active_door_pass = Some(ActiveDoorPass {
        door_index: DoorIndex::new(0).expect("valid door index"),
        direct: true,
        position_direct: true,
        steps: VecDeque::new(),
        preallocated_order_ids: Default::default(),
        triggers_fired: 0,
        current_action: crate::order::OrderType::default(),
        current_reverse: false,
        saved_action_state: None,
    });
    pc.actor.passing_door_directly = true;
    // Forecast preparation only treats the actor as mid door transit while
    // its position interface still holds the live door pointer.
    pc.element.sprite.position_iface.set_door_for_test(
        crate::position_interface::DoorHandle::new(0).expect("valid door index"),
    );

    // The original game's door-passing check observes the selected PassDoor
    // sequence command. Runtime door mirrors alone no longer arm forecast
    // preparation after the selected-command parity fix.
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(door_actor),
        crate::order::OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    {
        *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
        *direction = 1;
    } else {
        unreachable!("PassDoor fixture must be a movement element")
    }
    let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_sequence, 0);

    let building_sector = SectorNumber::new(8);
    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(7),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: MapPoint::new(0.0, 0.0),
            point_in: MapPoint::new(10.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(9),
            sector_in: building_sector,
            sector_out_index: crate::fast_find_grid::SectorIndex::new(2),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(0),
            point_out: MapPoint::new(100.0, 0.0),
            point_in: MapPoint::new(90.0, 0.0),
            ..Door::default()
        },
    ];
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    for (index, sector_number) in [building_sector, SectorNumber::new(7), SectorNumber::new(9)]
        .into_iter()
        .enumerate()
    {
        level.sector_number_map.insert(sector_number, index);
        level.sectors.push(GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: if index == 0 {
                SectorType::BUILDING
            } else {
                SectorType::MOTION | SectorType::AREA
            },
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
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![ClassEntry {
                source_file: "pa013_rng_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission script builds"),
    );
    engine.ai.global.ambush_points = vec![AmbushPoint {
        position: crate::ai::Position::default(),
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::default(),
        id: 0,
    }];
    engine.control.frame_counter = (quiet_owner.index() + 101) & 255;

    // Building scratch no longer burns BuildingExitGate eagerly: the
    // prepared forecast defers the gate selection draw until an AI statement
    // actually resolves it. Prove the fixture is armed by resolving the
    // door-passing actor's prepared forecast directly.
    let control_scratch = engine.build_sim_scratch(&assets);
    let (_, control_trace) = with_draw_trace(|| {
        control_scratch
            .ai_entity_views
            .get(&door_actor.index())
            .expect("door-passing actor has an AI entity view")
            .forecasted_destination
            .resolve(sim);
    });
    drop(control_scratch);
    assert!(
        control_trace.contains(&RngSite::BuildingExitGate),
        "the fixture must exercise BuildingExitGate when its prepared forecast is resolved"
    );

    let (_, empty_drain_trace) =
        with_draw_trace(|| engine.drain_pending_for_npc(sim, quiet_owner, &assets));
    assert!(
        !empty_drain_trace.contains(&RngSite::BuildingExitGate),
        "an empty common outbox drain must not build forecast scratch"
    );

    let (_, tail_trace) =
        with_draw_trace(|| engine.tick_npc_post_detection_tail_for_npc(sim, quiet_owner, &assets));
    assert!(
        !tail_trace.contains(&RngSite::BuildingExitGate),
        "due macro and retained Think work must not forecast an unrelated door-passing actor"
    );
}

#[test]
fn enemy_tick_data_uses_patrol_chiefs_committed_pass_door_side() {
    use crate::ai::AiState;
    use crate::coordinates::MapPoint;
    use crate::element::{Camp, Command};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::sector::SectorNumber;

    let mut engine = EngineInner::new();
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let minion_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    {
        let chief = engine.get_entity_mut(chief_id).unwrap();
        chief
            .element_data_mut()
            .set_position_map(MapPoint::new(814.0, 1110.2));
        chief.ai_controller_mut().unwrap().current_state = AiState::Default;
    }
    engine
        .get_entity_mut(minion_id)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .patrol_chief = Some(chief_id);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.script_domains.interactables.doors = vec![Door {
        door_type: DoorType::LiftLow,
        sector_out: SectorNumber::new(89),
        sector_in: SectorNumber::new(96),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(89),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(96),
        point_out: MapPoint::new(821.0, 1124.0),
        point_in: MapPoint::new(811.0, 1103.0),
        layer_out: 2,
        layer_in: 3,
        ..Door::default()
    }];
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(chief_id),
        crate::order::OrderType::WalkingStairs,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    {
        *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
        *direction = 0;
    } else {
        unreachable!("PassDoor fixture must be a movement element")
    }
    crate::sim_rng::with_seed(0xA013_0518, |sim| {
        // Tick data resolves the chief from live committed PassDoor state.
        let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
        engine
            .orders
            .sequence_manager
            .element_in_progress(pass_sequence, 0);
        let tick = engine.build_npc_tick_data(sim, minion_id, &assets);
        assert_eq!(tick.patrol_chief_position.x, 821.0);
        assert_eq!(tick.patrol_chief_position.y, 1124.0);
        assert_eq!(tick.patrol_chief_position.level, 2);
        assert_eq!(
            tick.patrol_chief_position.sector,
            crate::position_interface::SectorHandle::new(89).map(|handle| {
                handle.with_arena_index(crate::fast_find_grid::SectorIndex::new(89).unwrap())
            })
        );
    });
}

#[test]
fn optical_detection_uses_owner_relative_positions_and_spawned_current_fallback() {
    use crate::ai::AiLockFlags;
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus, Posture};

    fn observed(observer_before_target: bool, spawn_after_snapshot: bool) -> bool {
        let mut engine = EngineInner::new();
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));
        let (observer_id, initial_target) = if observer_before_target {
            let observer = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let target = (!spawn_after_snapshot)
                .then(|| engine.add_test_entity(make_test_pc(Posture::Upright)));
            (observer, target)
        } else {
            let target = engine.add_test_entity(make_test_pc(Posture::Upright));
            let observer = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            (observer, Some(target))
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let mut positions = engine.boundary_positions_snapshot();
        let target_id = initial_target
            .unwrap_or_else(|| engine.add_test_entity(make_test_pc(Posture::Upright)));
        if spawn_after_snapshot {
            complete_test_runtime_fixture(&mut engine, &mut assets);
        }
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the target character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
            unreachable!()
        };
        observer.element.active = true;
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 200;
        observer.npc.view_radius_base = 200;
        observer.npc.view_radius_goal = 200;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = EyeStatus::Stare;
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            shadow_seen_last_frame: true,
            ..Default::default()
        }];
        let ai = observer.npc.ai_brain.base_mut().unwrap();
        ai.me = observer_id.index();
        ai.locks_flag_field = AiLockFlags::FREEZE;

        let before = MapPoint::new(50.0, 0.0);
        let after = if spawn_after_snapshot {
            MapPoint::new(50.0, 0.0)
        } else {
            MapPoint::new(5_000.0, 0.0)
        };
        if positions.get(target_id).is_some() {
            positions[target_id] = Some(crate::entities::BoundaryPosition {
                map: before,
                world: crate::coordinates::WorldPoint3D::new(before.x, before.y, 0.0),
            });
        }
        let Entity::Pc(target) = engine.get_entity_mut(target_id).unwrap() else {
            unreachable!()
        };
        target.element.active = true;
        target.element.set_position_map(after);
        target.pc.life_points = 100;

        let sim = crate::sim_rng::test_context();
        let mut prepared = engine.prepare_npc_owner_pass();
        engine.tick_npc_owner_pass(&sim, &assets, &positions, &mut prepared, observer_id);

        engine
            .get_entity(observer_id)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[DetectableType::Enemy as usize][0]
            .seen_last_frame
    }

    assert!(
        observed(true, false),
        "an earlier observer must see a later moving target at its pre-movement position"
    );
    assert!(
        !observed(false, false),
        "a later observer must see an earlier moving target at its post-movement position"
    );
    assert!(
        observed(true, true),
        "a callback-spawned later target absent from the oracle must use its live current position"
    );
}

#[test]
fn inactive_building_viewer_runs_hearing_then_optics_while_outdoor_viewer_is_a_noop() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        // Slot 0 has Original creation order 31; frame 2 opens its
        // three-frame hearing cadence.
        engine.control.frame_counter = 2;
        let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let indoor_target_id =
            engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        let inactive_outdoor_id =
            engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        let runner_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        let building = crate::position_interface::SectorHandle::new(42).unwrap();
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists")
        else {
            panic!("inactive-viewer observer changed kind")
        };
        observer.element.active = true;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 300;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = crate::element::EyeStatus::Stare;
        if observer_inside {
            observer.element.set_sector(Some(building));
        }

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("inactive same-building target exists")
        else {
            panic!("inactive same-building target changed kind")
        };
        indoor_target.element.active = true;
        indoor_target
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(40.0, 0.0, 0.0));
        indoor_target
            .element
            .set_position_map(MapPoint::new(40.0, 0.0));
        indoor_target.element.set_sector(Some(building));
        indoor_target.pc.life_points = 100;

        let Entity::Pc(inactive_outdoor) = engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists")
        else {
            panic!("inactive outdoor target changed kind")
        };
        inactive_outdoor.element.active = true;
        inactive_outdoor
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(45.0, 0.0, 0.0));
        inactive_outdoor
            .element
            .set_position_map(MapPoint::new(45.0, 0.0));
        inactive_outdoor.pc.life_points = 100;

        let Entity::Pc(runner) = engine
            .get_entity_mut(runner_id)
            .expect("inactive-viewer runner exists")
        else {
            panic!("inactive-viewer runner changed kind")
        };
        runner.element.active = true;
        runner
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
        runner.element.set_position_map(MapPoint::new(55.0, 0.0));
        runner.pc.life_points = 100;

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(runner_id),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("same-building target exists after fixture")
        else {
            panic!("same-building target changed kind after fixture")
        };
        indoor_target.element.active = false;
        engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists after fixture")
            .element_data_mut()
            .active = false;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists after fixture")
        else {
            panic!("inactive-viewer observer changed kind after fixture")
        };
        observer.element.active = false;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("inactive-viewer observer has enemy AI");
        ai.base.me = observer_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![
            Detectable {
                element: Some(indoor_target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(inactive_outdoor_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: true,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(runner_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
        ];
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.maximal_detection_suspect = 777;

        crate::sim_rng::with_seed(0xA013_1A51, |sim| engine.tick_enemy_ai(sim, &assets));

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("inactive-viewer observer remains an NPC");
        let ai = observer
            .ai_brain
            .enemy()
            .expect("inactive-viewer observer retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Default);

        if observer_inside {
            assert_eq!(
                ai.base
                    .stimulus_queue
                    .iter()
                    .map(|stimulus| stimulus.stimulus_type)
                    .collect::<Vec<_>>(),
                vec![
                    StimulusType::EventHear,
                    StimulusType::EventView,
                    StimulusType::EventOutOfView,
                ],
                "an inactive building viewer must retain acoustic-before-optical FIFO order"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(
                !observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame,
                "a living inactive outdoor PC must stay in the list and produce a falling edge"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                0
            );
            assert_eq!(observer.maximal_detection_suspect, 0);
            assert_eq!(
                engine
                    .get_entity(indoor_target_id)
                    .and_then(Entity::actor_data)
                    .expect("inactive indoor target remains an actor")
                    .last_noise_volume,
                0,
                "retaining an inactive PC for same-building sight must not make it audible"
            );
        } else {
            assert!(
                ai.base.stimulus_queue.is_empty(),
                "the first detection-refresh check must make an inactive outdoor viewer a no-op"
            );
            assert_eq!(
                observer.detectable_lists[DetectableType::Enemy as usize].len(),
                3,
                "living inactive targets remain Enemy detectables until they die"
            );
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame);
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                999
            );
            assert_eq!(observer.maximal_detection_suspect, 777);
        }
    }
}

#[test]
fn inactive_npc_blip_detection_requires_door_or_building_eligibility() {
    use crate::element::{Camp, Entity};

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        // Slot 0 has Original creation order 31; frame 1 opens the common
        // modulo-16 blip cadence.
        engine.control.frame_counter = 1;
        let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("blipped inactive observer exists")
        else {
            panic!("blipped inactive observer changed kind")
        };
        observer.element.active = true;
        observer.element.blipped = true;
        observer.npc.life_points = 100;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        if observer_inside {
            observer.element.set_sector(Some(
                crate::position_interface::SectorHandle::new(42).unwrap(),
            ));
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("blip-viewing PC exists")
        else {
            panic!("blip-viewing PC changed kind")
        };
        pc.element.active = true;
        pc.pc.playable = true;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(20.0, 0.0));

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .get_entity_mut(observer_id)
            .expect("blipped observer exists after fixture")
            .element_data_mut()
            .active = false;

        crate::sim_rng::with_seed(0xA013_B11F, |sim| engine.tick_enemy_ai(sim, &assets));

        assert_eq!(
            engine
                .get_entity(observer_id)
                .expect("blipped observer survives tick")
                .element_data()
                .blipped,
            !observer_inside,
            "inactive building NPCs run blip detection; inactive outdoor NPCs do not"
        );
    }
}

#[test]
fn inactive_door_transit_viewer_runs_blip_and_hearing_then_skips_optics() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::position_interface::DoorHandle;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 17 makes modified
    // frame 48, opening both the three-frame hearing cadence and the
    // modulo-16 blip cadence.
    engine.control.frame_counter = 17;
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let runner_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists")
    else {
        panic!("door-transit observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(runner) = engine
        .get_entity_mut(runner_id)
        .expect("door-transit runner exists")
    else {
        panic!("door-transit runner changed kind")
    };
    runner.element.active = true;
    runner.pc.playable = true;
    runner.pc.life_points = 100;
    runner
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
    runner.element.set_position_map(MapPoint::new(55.0, 0.0));

    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(runner_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists after fixture")
    else {
        panic!("door-transit observer changed kind after fixture")
    };
    observer.element.active = false;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle::new(0).expect("valid door index"));
    observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(runner_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.maximal_detection_suspect = 777;
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("door-transit observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.max_visibility = 15;

    crate::sim_rng::with_seed(0xA013_D00F, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("door-transit observer survives tick")
    else {
        panic!("door-transit observer changed kind during tick")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy()
        .expect("door-transit observer retains enemy AI");
    assert!(
        !observer.element.blipped,
        "door transit passes the blip gate"
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventHear],
        "door transit passes acoustics but the sector-only optical gate rejects it"
    );
    assert!(observer.npc.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame);
    assert!(!observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
    assert_eq!(
        observer.npc.detection_suspects[DetectableType::Enemy as usize],
        999,
        "the door-only optical return must not scan or decay Enemy suspects"
    );
    assert_eq!(observer.npc.maximal_detection_suspect, 0);
    assert_eq!(ai.base.max_visibility, 0);
}

#[test]
#[should_panic(expected = "Enemy detectable target 999999 for NPC 0 is missing")]
fn mixed_enemy_walk_rejects_missing_detectable_target_with_context() {
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, _, _) = mixed_enemy_fifo_fixture(true);
    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("missing-target observer retains NPC state");
    observer.detectable_lists[DetectableType::Enemy as usize][0].element =
        Some(EntityId::Soldier(crate::entity_id::SoldierId(999_999)));

    crate::sim_rng::with_seed(0xA013_BAD1, |sim| engine.tick_enemy_ai(sim, &assets));
}

#[test]
#[should_panic(expected = "eligible civilian NPC 0 has no FriendlyAi brain during detection")]
fn mixed_enemy_walk_rejects_missing_observer_ai_with_context() {
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("missing-AI civilian exists")
    else {
        panic!("missing-AI observer changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Lacklandists;
    civilian.npc.life_points = 100;
    civilian.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("missing-AI target exists")
    else {
        panic!("missing-AI target changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("missing-AI civilian exists after fixture completion")
    else {
        panic!("missing-AI observer changed kind after fixture completion")
    };
    civilian.npc.ai_brain = crate::element::AiBrain::None;
    crate::sim_rng::with_seed(0xA013_BAD2, |sim| engine.tick_enemy_ai(sim, &assets));
}

#[test]
#[should_panic(expected = "eligible soldier NPC 0 has no EnemyAi brain during detection")]
fn mixed_enemy_walk_rejects_friendly_ai_on_a_soldier() {
    use crate::element::{AiBrain, Camp, Entity};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("wrong-AI soldier exists")
    else {
        panic!("wrong-AI observer changed kind")
    };
    soldier.element.active = true;
    soldier.npc.life_points = 100;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("wrong-AI soldier survives fixture setup")
    else {
        panic!("wrong-AI observer changed kind after fixture")
    };
    soldier.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        soldier_id.index(),
    )));
    let _ = engine.enemy_optical_viewer_context_for_test(soldier_id);
}

#[test]
fn mixed_enemy_cleanup_removes_negative_life_targets() {
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("negative-life PC target exists")
    else {
        panic!("negative-life PC target changed kind")
    };
    pc.pc.life_points = -5;
    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("negative-life soldier target exists")
    else {
        panic!("negative-life soldier target changed kind")
    };
    royalist.npc.life_points = -7;

    crate::sim_rng::with_seed(0xA013_DEAD, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("negative-life observer retains NPC state");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize].is_empty(),
        "detectable cleanup must use life <= 0 for PCs and soldiers"
    );
}

#[test]
fn blipped_lacklandist_in_door_transit_is_inside_for_the_pre_cadence_gate() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};
    use crate::position_interface::DoorHandle;

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    // Modified frame 34 keeps blip/NPC cadence closed while opening the
    // Lacklandist PC cadence.
    engine.control.frame_counter = 3;

    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("door-transit rear target exists")
    else {
        panic!("door-transit rear target changed kind")
    };
    royalist
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-120.0, 0.0, 0.0));
    royalist
        .element
        .set_position_map(MapPoint::new(-120.0, 0.0));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit optical observer exists")
    else {
        panic!("door-transit optical observer changed kind")
    };
    observer.element.blipped = true;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle::new(0).expect("valid door index"));
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;

    assert!(
        !(engine.control.frame_counter + observer_id.index() + 31)
            .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
        "fixture must keep NPC blip auto-reveal closed"
    );
    crate::sim_rng::with_seed(0xA013_D016, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("door-transit optical observer retains NPC state");
    let view_targets = observer
        .ai_brain
        .base()
        .expect("door-transit optical observer retains AI state")
        .stimulus_queue
        .iter()
        .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventView)
        .map(|stimulus| {
            let StimulusInfo::Human(target) = stimulus.info else {
                panic!("door-transit VIEW lost its human target")
            };
            target
        })
        .collect::<Vec<_>>();
    assert_eq!(
        view_targets,
        vec![crate::ai::AiEntityHandle::new(pc_id.index())],
        "the door pointer passes the PC blip gate, but must not fabricate a same-building handle that reveals the rear soldier"
    );
}

#[test]
fn civilian_enemy_optics_uses_the_common_npc_walk() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("optical civilian exists")
    else {
        panic!("optical civilian changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Lacklandists;
    civilian.npc.life_points = 100;
    civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
        crate::ai_friendly::FriendlyAi::new(civilian_id.index()),
    ));
    civilian.element.set_direction_instantly(4);
    civilian.npc.view_direction = [1.0, 0.0];
    civilian.npc.view_radius = 300;
    civilian.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    civilian.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("civilian target exists")
    else {
        panic!("civilian target changed kind")
    };
    pc.element.active = true;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(80.0, 0.0));
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("optical civilian exists after fixture")
    else {
        panic!("optical civilian changed kind after fixture")
    };
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("optical civilian has FriendlyAi");
    ai.me = civilian_id.index();
    ai.locks_flag_field = AiLockFlags::BUSY;
    ai.got_the_beggar_trick = true;
    civilian.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    civilian.npc.detection_suspects[DetectableType::Enemy as usize] = 999;

    crate::sim_rng::with_seed(0xA013_C1A0, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(civilian_id)
        .and_then(Entity::ai_controller)
        .expect("optical civilian retains FriendlyAi");
    assert_eq!(ai.stimulus_queue.len(), 1);
    assert_eq!(ai.stimulus_queue[0].stimulus_type, StimulusType::EventView);
    assert_eq!(
        ai.stimulus_queue[0].info,
        StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
    );
    // The PC stands 80 units east of the civilian's eye point (beyond both
    // the very-close and halfcircle radii), so the committed sharpness is
    // BASE_VIEW_SPEED × DETECTION_FREQUENCY_ENEMY_PC × the distance curve,
    // truncated to an integer like the engine does.
    let expected_sharpness = (f32::from(crate::ai_vision::BASE_VIEW_SPEED)
        * crate::ai_vision::DETECTION_FREQUENCY_ENEMY_PC as f32
        * crate::ai_vision::distance_sharpness(80.0 * 80.0, 300.0))
        as u32;
    assert!(expected_sharpness > 0);
    assert_eq!(
        ai.max_visibility, expected_sharpness,
        "the shared NPC maximum must be published through FriendlyAi too"
    );
}
