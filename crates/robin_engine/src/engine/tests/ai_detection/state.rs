use super::*;

#[test]
fn quiet_pc_noise_refresh_preserves_the_previous_hearing_box() {
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
        panic!("noise PC changed kind")
    };
    pc_entity.element.active = true;
    pc_entity
        .element
        .set_position_map(MapPoint::new(100.0, 200.0));

    // An unclassified animation reaches the noise refresh's common tail:
    // volume zero and a +/-100 box around the current position.
    engine.refresh_pc_produced_noise_for_with_order(pc, OrderType::Invalid);
    let initial_box = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor")
        .hear_noise_box;
    assert_eq!(
        initial_box,
        crate::coordinates::MapBBox::from_coords(0.0, 100.0, 200.0, 300.0)
    );

    // Original's breath arm updates the noise position and volume, then
    // returns before rebuilding the heard-noise bounds. Preserve both halves of
    // that deliberately inconsistent state after the PC moves.
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).unwrap() else {
        unreachable!()
    };
    pc_entity
        .element
        .set_position_map(MapPoint::new(210.0, 220.0));
    engine.refresh_pc_produced_noise_for_with_order(pc, OrderType::WaitingUpright);

    let actor = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor");
    let noise = actor
        .produced_noise
        .expect("quiet refresh still publishes the current noise record");
    assert_eq!(noise.volume, 15);
    assert_eq!((noise.origin.x, noise.origin.y), (210.0, 220.0));
    assert_eq!(actor.hear_noise_box, initial_box);
    assert!(
        !actor
            .hear_noise_box
            .contains_point(MapPoint::new(210.0, 220.0))
    );
}

#[test]
fn patrol_member_thinks_before_the_chief_applies_its_direction() {
    use crate::ai::{AiState, PathHistoryEntry, PathId, PatrolPath, Position, Substate};

    let mut engine = EngineInner::new();
    let chief = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let member = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [chief, member] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("patrol soldier exists")
        else {
            panic!("patrol soldier changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("patrol soldier has enemy AI")
            .base
            .me = id.index();
    }
    let chief_position = Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    };
    let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
        unreachable!()
    };
    chief_entity
        .element
        .set_position_map(MapPoint::new(chief_position.x, chief_position.y));
    chief_entity.element.sprite.position_iface.set_move_box(
        crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-2.0, -2.0),
            crate::coordinates::MapVec::new(2.0, 2.0),
        ),
    );
    let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
    chief_ai.current_state = AiState::Default;
    chief_ai.theoretical_patrol = vec![member];
    chief_ai.patrol = vec![member];
    chief_ai.patrol_path = Some(PatrolPath {
        hiking_path_index: PathId::new(0).unwrap(),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        size: 1,
        history: vec![
            PathHistoryEntry {
                position: Position::default(),
                direction: 9,
                distance: 0,
            },
            PathHistoryEntry {
                position: chief_position,
                direction: 9,
                distance: 100,
            },
        ],
    });
    let Entity::Soldier(member_entity) = engine.get_entity_mut(member).unwrap() else {
        unreachable!()
    };
    member_entity
        .element
        .set_position_map(MapPoint::new(500.0, 0.0));
    member_entity.element.set_direction_instantly(3);
    let member_ai = member_entity.npc.ai_brain.base_mut().unwrap();
    member_ai.patrol_chief = Some(chief);
    member_ai.current_state = AiState::Default;
    member_ai.current_substate = Substate::DefaultPatrolEnrouteWaiting;
    engine.control.frame_counter = 0;
    let mut positions = engine.boundary_positions_snapshot();

    crate::sim_rng::with_seed(0xA013_7A70, |sim| {
        engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
    });

    let member_entity = engine.get_entity(member).unwrap();
    let member_ai = member_entity.ai_controller().unwrap();
    assert_ne!(
        member_ai.current_substate,
        Substate::DefaultPatrolEnrouteWaiting,
        "patrol coordinate Think must leave waiting before direction is applied"
    );
    assert_eq!(member_ai.patrol_direction, 9);
    assert_eq!(
        member_entity.element_data().direction(),
        3,
        "CALL_PATROL_COORDINATE must leave the waiting substate before patrol-direction selection; applying direction first would emit a face action"
    );
}

#[test]
fn inactive_dead_patrol_chief_still_records_eligible_history() {
    use crate::ai::{AiState, PathId, PatrolPath};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(chief_entity) = engine.get_entity_mut(chief).unwrap() else {
        unreachable!()
    };
    chief_entity.element.active = false;
    chief_entity.npc.life_points = 0;
    let chief_ai = chief_entity.npc.ai_brain.base_mut().unwrap();
    chief_ai.current_state = AiState::Default;
    chief_ai.patrol = vec![member];
    chief_ai.patrol_path = Some(PatrolPath {
        hiking_path_index: PathId::new(0).unwrap(),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        size: 1,
        history: Vec::new(),
    });
    engine.control.frame_counter = 1;
    let mut positions = engine.boundary_positions_snapshot();

    crate::sim_rng::with_seed(0xA013_DEAD, |sim| {
        engine.tick_patrol_coordination_for_npc(sim, &assets, chief, &positions)
    });

    assert_eq!(
        engine
            .get_entity(chief)
            .unwrap()
            .ai_controller()
            .unwrap()
            .patrol_path
            .as_ref()
            .unwrap()
            .history
            .len(),
        1,
        "patrol refresh does not gate the per-frame history write on chief activity or life"
    );
}

#[test]
#[should_panic(expected = "missing its required AI controller while applying recovery state")]
fn think_with_drain_rejects_a_soldier_missing_its_required_ai() {
    use crate::ai::{AiContext, AiPerTickData, Stimulus, StimulusType};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));

    engine.dispatch_think_with_drain(
        sim,
        npc_id,
        &Stimulus::new(StimulusType::EventDone),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        &LevelAssets::new(),
    );
}

#[test]
fn ambush_refresh_drains_look_sidewards_before_next_tail_phase() {
    use crate::ai::{AiState, AmbushPoint, Position, Substate};
    use crate::ai_enemy::AmbushPointStatus;
    use crate::element::Command;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("ambush owner exists")
    else {
        panic!("ambush owner changed kind")
    };
    soldier.element.active = true;
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(0);
    let enemy = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("ambush owner has enemy AI");
    enemy.base.current_state = AiState::Seeking;
    enemy.base.current_substate = Substate::SeekingSeekpoint;
    enemy.soldier_profile_iq = 100;
    enemy.ambush_point_status = vec![AmbushPointStatus::Near];
    engine.ai.global.ambush_points = vec![AmbushPoint {
        position: Position {
            x: 10.0,
            y: 0.0,
            ..Position::default()
        },
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::new(10.0, 0.0, 32.0),
        id: 0,
    }];

    engine.tick_refresh_ambush_points_for_npc(sim, npc_id, &assets);

    let enemy = engine
        .get_entity(npc_id)
        .and_then(Entity::enemy_ai)
        .expect("ambush owner retains enemy AI");
    assert_eq!(
        enemy.base.current_substate,
        Substate::SeekingSeekpointCheckingAmbushPoint
    );
    assert!(enemy.base.outbox.actor.look_sidewards.is_none());
    assert!(
        [Command::LookLeft, Command::LookRight]
            .into_iter()
            .any(|command| engine.actor_command(npc_id) == command
                || engine
                    .orders
                    .sequence_manager
                    .element_is_about_to_be_launched(npc_id, command)),
        "ambush-point refresh must launch a sideways look before deafness/busy checks"
    );
}

#[test]
fn normal_timer_uses_unsigned_wrapped_overflow_guard() {
    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = u32::MAX - 10;
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .expect("overflow-timer owner has AI");
    ai.timer_is_running = true;
    ai.when_does_timer_ring = u32::MAX - 5;
    ai.substate_at_last_timer_launch = ai.current_substate;

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("overflow-timer owner retains AI");
    assert!(
        !ai.timer_is_running || ai.when_does_timer_ring != u32::MAX - 5,
        "the wrapped million-frame guard must consume the apparently-future timer"
    );
}

#[test]
fn retained_fifo_stops_when_first_think_acquires_busy_lock() {
    use crate::ai::{AiLockFlags, Stimulus, StimulusType};
    use crate::element::Posture;

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("FIFO owner exists") else {
        panic!("FIFO owner changed kind")
    };
    soldier.element.publish_order_posture(Posture::OnLadder);
    let ai = soldier.npc.ai_brain.base_mut().expect("FIFO owner has AI");
    ai.locks_flag_field = AiLockFlags::empty();
    ai.stimulus_queue = vec![
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::new(StimulusType::EventCouldntReachPoint),
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::new(StimulusType::EventTimer),
    ];

    engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("FIFO owner retains AI");
    assert!(ai.locks_flag_field.contains(AiLockFlags::BUSY));
    assert_eq!(
        ai.stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventAfterScriptGoOn,
            StimulusType::EventTimer,
            StimulusType::EventCouldntReachPoint,
        ],
        "the lock check must preserve both a later duplicate marker and its suffix before the causal retry"
    );
}

#[test]
fn panic_generated_reachpoint_precedes_retained_panic_sibling_and_draws_twice() {
    use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());

    // Keep the first panic segment inside an open grid.  There are no door
    // seek records, so the first retained EVENT_PANIC must enter the
    // original-game no-door branch and recursively dispatch the reach-point event.
    engine.world.fast_grid_mut().size_map(64, 64);
    engine.world.fast_grid_mut().allocate_layers(1);
    let sector = crate::position_interface::SectorHandle::new(1).unwrap();
    let Entity::Civilian(civilian) = engine.get_entity_mut(npc_id).unwrap() else {
        panic!("retained panic owner changed kind")
    };
    civilian.element.active = true;
    civilian
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(1000.0, 1000.0, 0.0));
    civilian.element.set_layer(0);
    civilian.element.set_sector(Some(sector));
    civilian
        .element
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-2.0, -2.0),
            crate::coordinates::MapVec::new(2.0, 2.0),
        ));
    civilian.npc.life_points = 100;
    let ai = civilian.npc.ai_brain.base_mut().unwrap();
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultEnroute;
    ai.script_locked = false;
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventTimer.into());
    ai.stimulus_queue = vec![
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::with_position(
            StimulusType::EventPanic,
            Position {
                x: 900.0,
                y: 1000.0,
                sector: Some(sector),
                level: 0,
            },
        ),
        Stimulus::new(StimulusType::EventAfterScriptGoOn),
        Stimulus::with_position(
            StimulusType::EventPanic,
            Position {
                x: 1100.0,
                y: 1000.0,
                sector: Some(sector),
                level: 0,
            },
        ),
    ];

    let (_, draws) =
        with_draw_trace(|| engine.tick_ai_queued_stimuli_for_npc(sim, npc_id, &assets));

    assert_eq!(draws, vec![RngSite::AiPanic, RngSite::AiPanic]);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .expect("retained panic owner keeps AI");
    let events = ai
        .ai_log
        .iter()
        .filter(|line| line.line_type == crate::ai::LogLineType::Event)
        .map(|line| line.info)
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            StimulusType::EventAfterScriptGoOn as u16,
            StimulusType::EventPanic as u16,
            StimulusType::EventReachPoint as u16,
            StimulusType::EventTimer as u16,
            StimulusType::EventPanic as u16,
        ],
        "Panic's direct recursive Think must precede both an existing self backlog and the retained sibling"
    );
}

#[test]
fn synchronous_look_there_refreshes_only_at_the_receivers_creation_slot() {
    use crate::ai::{
        AiState, CrossNpcAction, Hint, Position, StimulusInfo, StimulusType, Substate,
    };
    use crate::element::{Camp, Entity, EyeStatus};

    fn observe(receiver_before_source: bool) -> (f32, f32, EyeStatus) {
        let mut engine = EngineInner::new();
        let source = make_test_ai_soldier(Camp::Lacklandists);
        let receiver = make_test_ai_soldier(Camp::Lacklandists);
        let (source_id, receiver_id) = if receiver_before_source {
            let receiver_id = engine.add_test_entity(receiver);
            let source_id = engine.add_test_entity(source);
            (source_id, receiver_id)
        } else {
            let source_id = engine.add_test_entity(source);
            let receiver_id = engine.add_test_entity(receiver);
            (source_id, receiver_id)
        };
        for id in [source_id, receiver_id] {
            let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
                panic!("LOOKTHERE test NPC changed kind")
            };
            soldier.element.active = true;
            soldier.npc.life_points = 100;
            soldier.element.set_direction_instantly(4);
            soldier.npc.direction_old = 4;
        }
        {
            let receiver = engine
                .get_entity_mut(receiver_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap();
            // Recipient selection happened while this soldier was eligible,
            // but an earlier synchronous callback then changed its state.
            // CALL_LOOKTHERE itself is unconditional in the Original.
            receiver.current_state = AiState::Seeking;
            receiver.current_substate = Substate::SeekingGroupCalledByOfficer;
        }
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let hint = Hint {
            seek_point: Position {
                x: 0.0,
                y: 100.0,
                sector: None,
                level: 0,
            },
            seek_flags: 0,
            who_tells_me: crate::ai::AiEntityHandle::new(source_id.index()),
        };
        engine
            .get_entity_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap()
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::SendStimulus {
                target: receiver_id.index(),
                stimulus_type: StimulusType::CallLookThere,
                info: StimulusInfo::Hint(hint),
                fallback_to_sender: None,
                to_whole_patrol: false,
            });

        let mut positions = engine.boundary_positions_snapshot();
        crate::sim_rng::with_seed(0xA013_1007, |sim| {
            if receiver_before_source {
                engine.refresh_npc_view_for_npc(receiver_id, &positions);
                engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
            } else {
                engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
                engine.refresh_npc_view_for_npc(receiver_id, &positions);
            }
        });

        let receiver = engine
            .get_entity(receiver_id)
            .and_then(Entity::npc_data)
            .unwrap();
        let receiver_ai = receiver.ai_brain.base().unwrap();
        assert_eq!(receiver_ai.current_state, AiState::Wondering);
        assert_eq!(receiver_ai.current_substate, Substate::WonderingWatching);
        (
            receiver.view_angle,
            receiver.view_angle_step,
            receiver.eye_status,
        )
    }

    let earlier = observe(true);
    let later = observe(false);
    assert_eq!(earlier.2, EyeStatus::Stare);
    assert_eq!(later.2, EyeStatus::Stare);
    assert!(
        earlier.0.abs() < f32::EPSILON,
        "an earlier receiver already spent its one view refresh before LOOKTHERE"
    );
    assert!(
        (later.0 - later.1).abs() < f32::EPSILON,
        "a later receiver must advance its stateful stare exactly once at its own slot"
    );
}

#[test]
fn frozen_all_does_not_defer_fit_again_recovery_effects() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, Entity, EyeStatus, Posture};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    engine
        .get_entity_mut(npc_id)
        .unwrap()
        .element_data_mut()
        .active = true;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].wake_up = 1;

    let Entity::Soldier(npc) = engine.get_entity_mut(npc_id).unwrap() else {
        unreachable!()
    };
    npc.element.publish_order_posture(Posture::Lying);
    npc.human.unconscious = true;
    npc.human.concussion_of_the_brain = crate::combat::CONCUSSION_WAKEUP_THRESHOLD;
    npc.human.concussion_healing_timeout = 0;
    npc.npc.life_points = 100;
    npc.npc.eye_status = EyeStatus::Closed;
    npc.npc.view_radius = 0;
    npc.npc.view_radius_base = 173;
    npc.npc.view_radius_goal = 173;
    npc.npc.view_longrange_radius_factor = 1.0;
    let ai = npc.npc.ai_brain.base_mut().unwrap();
    ai.me = npc_id.index();
    ai.current_state = AiState::Sleeping;
    ai.current_substate = Substate::SleepingUnconscious;

    let observer = engine
        .get_entity_mut(observer_id)
        .unwrap()
        .npc_data_mut()
        .unwrap();
    observer.detectable_lists[crate::element::DetectableType::Body as usize].push(
        crate::element::Detectable {
            element: Some(npc_id),
            detectable_type: crate::element::DetectableType::Body,
            ..Default::default()
        },
    );

    engine.set_actors_frozen(true);
    let mut positions = engine.boundary_positions_snapshot();
    crate::sim_rng::with_seed(0x0A01_3F20, |sim| {
        engine.tick_actor_owner_envelopes(sim, &assets, &positions)
    });

    let npc = engine.get_entity(npc_id).unwrap();
    assert_eq!(npc.npc_data().unwrap().eye_status, EyeStatus::LookForward);
    assert!(!npc.human_data().unwrap().unconscious);
    let ai = npc.ai_controller().unwrap();
    assert!(!ai.outbox.recovery.inform_resurrection);
    assert_eq!(ai.outbox.recovery.set_eye_status, None);
    assert!(
        engine
            .get_entity(observer_id)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Body as usize]
            .is_empty(),
        "FIT_AGAIN's resurrection fan-out is inline even while FrozenAll skips the NPC tail"
    );
}

#[test]
fn restored_quit_lose_quit_fifo_commits_unconscious_eyes_inline() {
    use crate::ai::{Stimulus, StimulusType};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap();
    ai.outbox.detection.stimuli = vec![
        Stimulus::new(StimulusType::EventQuitSwordfight),
        Stimulus::new(StimulusType::EventLoseConsciousness),
        Stimulus::new(StimulusType::EventQuitSwordfight),
    ];

    crate::sim_rng::with_seed(0xA013_105E, |sim| {
        engine.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, &assets, None, None)
    });

    let entity = engine.get_entity(npc_id).unwrap();
    assert_eq!(
        entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious,
        "the middle LOSE_CONSCIOUSNESS Think must publish its eye write despite the surrounding restored FIFO prefix/suffix"
    );
    assert_eq!(
        entity
            .ai_controller()
            .unwrap()
            .outbox
            .recovery
            .set_eye_status,
        None,
        "the restored FIFO must not strand its synchronous view-status write"
    );
}

#[test]
fn wake_blinks_apply_inline_at_the_waker_slot_for_both_producers() {
    use crate::ai::{AiState, StimulusType, Substate};
    use crate::combat::ConcussionOutcome;
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus, Posture};

    type BlinkState = (bool, bool);

    fn observe(waker_before_observer: bool, natural: bool) -> (BlinkState, BlinkState) {
        let mut engine = EngineInner::new();
        engine
            .ai
            .global
            .soldier_camps
            .extend([Camp::Royalists, Camp::Lacklandists]);
        let waker = make_test_ai_soldier(Camp::Royalists);
        let observer = make_test_ai_soldier(Camp::Lacklandists);
        let (waker_id, observer_id) = if waker_before_observer {
            (
                engine.add_test_entity(waker),
                engine.add_test_entity(observer),
            )
        } else {
            let observer_id = engine.add_test_entity(observer);
            let waker_id = engine.add_test_entity(waker);
            (waker_id, observer_id)
        };
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles
            .soldiers
            .resize_with(1, crate::profiles::SoldierProfile::default);
        profiles.soldiers[0].wake_up = 1;

        let Entity::Soldier(waker) = engine.get_entity_mut(waker_id).unwrap() else {
            unreachable!()
        };
        waker.element.active = true;
        waker.element.publish_order_posture(Posture::Lying);
        waker.npc.life_points = 100;
        waker.npc.eye_status = EyeStatus::Closed;
        let ai = waker.npc.ai_brain.base_mut().unwrap();
        ai.script_locked = false;
        ai.current_state = AiState::Sleeping;
        ai.current_substate = Substate::SleepingUnconscious;
        if natural {
            waker.human.unconscious = true;
            waker.human.concussion_of_the_brain = crate::combat::CONCUSSION_WAKEUP_THRESHOLD;
            waker.human.concussion_healing_timeout = 0;
        }
        let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
            unreachable!()
        };
        observer.element.active = false;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(waker_id),
            detectable_type: DetectableType::Enemy,
            seen_now: true,
            seen_last_frame: true,
            ..Detectable::default()
        }];

        if natural {
            engine.tick_concussion_healing(&assets);
        } else {
            engine
                .orders
                .pending_concussion_side_effects
                .push((waker_id, ConcussionOutcome::WokeUp));
            crate::sim_rng::with_seed(0x0A01_3B11, |sim| {
                engine.drain_pending_concussion_side_effects(sim, &assets)
            });
        }
        assert!(
            engine
                .get_entity(waker_id)
                .and_then(Entity::ai_controller)
                .unwrap()
                .outbox
                .detection
                .stimuli
                .iter()
                .any(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain),
            "producer natural={natural}, waker_before_observer={waker_before_observer}, unconscious={}",
            engine
                .get_entity(waker_id)
                .and_then(Entity::human_data)
                .unwrap()
                .unconscious
        );

        let mut positions = engine.boundary_positions_snapshot();
        crate::sim_rng::with_seed(0x0A01_3B12, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });

        let snapshot = |engine: &EngineInner| {
            let observer = engine.get_entity(observer_id).unwrap();
            let detectable =
                &observer.npc_data().unwrap().detectable_lists[DetectableType::Enemy as usize][0];
            (detectable.seen_now, detectable.seen_last_frame)
        };
        let first_slot = snapshot(&engine);

        crate::sim_rng::with_seed(0x0A01_3B13, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });
        let next_slot = snapshot(&engine);

        (first_slot, next_slot)
    }

    for natural in [true, false] {
        assert_eq!(observe(true, natural), ((false, false), (false, false)));
        assert_eq!(
            observe(false, natural),
            ((false, false), (false, false)),
            "BlinkEnemy must mutate an already-visited opposing observer inline at the later waker's slot"
        );
    }
}

#[test]
fn nonserialized_primary_target_multiplicity_starts_empty_after_restore() {
    use crate::ai::{AiEntityHandle, AiState, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let attacker_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Soldier(attacker) = engine
        .get_entity_mut(attacker_id)
        .expect("restored attacker exists")
    else {
        panic!("restored attacker changed kind")
    };
    let ai = attacker
        .npc
        .ai_brain
        .enemy_mut()
        .expect("restored attacker has enemy AI");
    ai.base.me = attacker_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(target_id.index()));

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _prepared = engine.prepare_npc_owner_pass();

    assert!(engine.ai.global.primary_target_multiplicity_initialized);
    assert!(
        engine
            .ai
            .global
            .primary_target_multiplicity_scratch
            .is_empty(),
        "restored swordfight controllers must not synthesize nonserialized actor counters"
    );
}

#[test]
fn royalist_blip_auto_reveal_obeys_the_common_sixteen_frame_cadence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist blip observer exists")
    else {
        panic!("Royalist blip observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // Slot 0 has Original creation order 31. Frame 2 is closed; frame 1 below
    // is open for the common modulo-16 blip cadence.
    engine.control.frame_counter = 2;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives closed cadence")
            .element_data()
            .blipped
    );

    engine.control.frame_counter = 1;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        !engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives open cadence")
            .element_data()
            .blipped
    );
}

#[test]
fn closed_cadence_cannot_reuse_visibility_blocked_by_eyes_blip_or_guard() {
    use crate::ai::{AlertLevel, StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity, EyeStatus};

    #[derive(Clone, Copy, Debug)]
    enum Blocker {
        BlindEyes,
        BlippedViewer,
        GuardedPc,
    }

    for blocker in [
        Blocker::BlindEyes,
        Blocker::BlippedViewer,
        Blocker::GuardedPc,
    ] {
        let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
        // Modified frame 33 keeps both the PC cadence and the common blip
        // cadence closed.
        engine.control.frame_counter = 2;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("closed-cadence observer exists")
        else {
            panic!("closed-cadence observer changed kind")
        };
        observer.npc.eye_status = if matches!(blocker, Blocker::BlindEyes) {
            EyeStatus::Closed
        } else {
            EyeStatus::LookForward
        };
        observer.element.blipped = matches!(blocker, Blocker::BlippedViewer);
        observer
            .npc
            .ai_brain
            .base_mut()
            .expect("closed-cadence observer retains AI state")
            .current_music_alert_status = AlertLevel::Green;
        let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
            .iter_mut()
            .find(|detectable| detectable.element == Some(pc_id))
            .expect("closed-cadence observer tracks PC");
        detectable.last_visibility = 1.0;
        detectable.seen_now = true;
        detectable.seen_last_frame = !matches!(blocker, Blocker::GuardedPc);

        if matches!(blocker, Blocker::GuardedPc) {
            let Entity::Pc(pc) = engine
                .get_entity_mut(pc_id)
                .expect("closed-cadence guarded PC exists")
            else {
                panic!("closed-cadence guarded target changed kind")
            };
            pc.pc.guard = Some(observer_id);
        }

        assert!(
            !(engine.control.frame_counter + observer_id.index() + 31)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_PC)
        );
        crate::sim_rng::with_seed(0xA013_1A00 + blocker as u64, |sim| {
            engine.tick_enemy_ai(sim, &assets)
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("closed-cadence observer retains NPC state");
        let detectable = observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|detectable| detectable.element == Some(pc_id))
            .expect("blocked PC remains detectable");
        assert_eq!(
            (
                detectable.seen_now,
                detectable.seen_last_frame,
                detectable.last_visibility,
            ),
            (false, false, 0.0),
            "{blocker:?} must invalidate cached visibility before cadence"
        );
        let ai = observer
            .ai_brain
            .base()
            .expect("closed-cadence observer retains AI state");
        let out_of_view_targets = ai
            .stimulus_queue
            .iter()
            .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventOutOfView)
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("closed-cadence OUTOFVIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if matches!(blocker, Blocker::GuardedPc) {
            Vec::new()
        } else {
            vec![crate::ai::AiEntityHandle::new(pc_id.index())]
        };
        assert_eq!(
            out_of_view_targets, expected,
            "{blocker:?} must preserve the Original falling-edge semantics"
        );
    }
}

#[test]
fn closed_cadence_cached_visibility_contributes_to_maximal_sharpness() {
    use crate::ai::AlertLevel;
    use crate::element::{DetectableType, Entity, EyeStatus};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer slot 0 has Original creation order 31, so modified frame 33
    // closes the modulo-2 PC cadence.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("closed-cadence observer exists")
    else {
        panic!("closed-cadence observer changed kind")
    };
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("closed-cadence observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    observer.npc.eye_status = EyeStatus::LookForward;
    for (kind, list) in observer.npc.detectable_lists.iter_mut().enumerate() {
        if kind == DetectableType::Enemy as usize {
            list.retain(|detectable| detectable.element == Some(pc_id));
        } else {
            list.clear();
        }
    }
    let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
        .first_mut()
        .expect("closed-cadence observer tracks PC");
    detectable.last_visibility = 1.0;
    detectable.seen_now = true;
    detectable.seen_last_frame = true;

    crate::sim_rng::with_seed(0xA013_1A10, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::ai_controller)
        .expect("closed-cadence observer retains AI state");
    assert_eq!(
        ai.max_visibility,
        u32::from(crate::ai_vision::BASE_VIEW_SPEED),
        "Original maximizes integer sharpness after cached visibility reuse"
    );
}

#[test]
fn bonus_refresh_discovered_is_live_bonus_owned_freeze_safe_and_rng_free() {
    use crate::sim_rng::with_draw_trace;

    let mut engine = EngineInner::new();
    engine.ai.standard_view_polygon_radius = 100;
    let bonus_before = engine.add_test_entity(make_discovery_bonus(10.0));
    let hole = engine.add_test_entity(make_discovery_bonus(5_000.0));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let bonus_after = engine.add_test_entity(make_discovery_bonus(10.0));
    let scroll = engine.add_test_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectScroll,
    ));
    let projectile = engine.add_test_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectProjectile,
    ));
    let net = engine.add_test_entity(make_blipped_non_bonus(
        crate::element::ElementKind::ObjectNet,
    ));
    engine.remove_entity(hole);
    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("discovery PC exists") else {
        panic!("discovery PC changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(0.0, 0.0));
    engine.set_actors_frozen(true);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let (_, trace) = with_draw_trace(|| run_owner_envelopes(&mut engine, &assets));

    assert!(
        trace.is_empty(),
        "bonus discovery must not consume simulation RNG"
    );
    assert!(
        !engine
            .get_entity(bonus_before)
            .unwrap()
            .element_data()
            .blipped
    );
    assert!(
        !engine
            .get_entity(bonus_after)
            .unwrap()
            .element_data()
            .blipped
    );
    for id in [scroll, projectile, net] {
        assert!(
            engine.get_entity(id).unwrap().element_data().blipped,
            "only Entity::Bonus owns discovery refresh; {id:?} was revealed"
        );
    }
}

#[test]
fn bonus_refresh_discovered_uses_live_pc_eligibility_and_original_shoulders_factor() {
    fn discovered(
        posture: crate::element::Posture,
        x: f32,
        active: bool,
        life: i16,
        unconscious: bool,
    ) -> bool {
        let mut engine = EngineInner::new();
        engine.ai.standard_view_polygon_radius = 100;
        let pc_id = engine.add_test_entity(make_test_pc(posture));
        let bonus_id = engine.add_test_entity(make_discovery_bonus(x));
        let Entity::Pc(pc) = engine.get_entity_mut(pc_id).unwrap() else {
            unreachable!()
        };
        pc.element.active = active;
        pc.pc.life_points = life;
        pc.human.unconscious = unconscious;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(0.0, 0.0));
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let eye_z = engine
            .get_entity(pc_id)
            .unwrap()
            .compute_eyes_point(None)
            .unwrap()
            .z;
        engine
            .get_entity_mut(bonus_id)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, eye_z));
        engine.refresh_bonus_discovered_for(&assets, bonus_id);
        !engine.get_entity(bonus_id).unwrap().element_data().blipped
    }

    assert!(!discovered(
        crate::element::Posture::Upright,
        105.0,
        true,
        100,
        false
    ));
    assert!(discovered(
        crate::element::Posture::OnShoulders,
        105.0,
        true,
        100,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        false,
        100,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        true,
        0,
        false
    ));
    assert!(!discovered(
        crate::element::Posture::Upright,
        10.0,
        true,
        100,
        true
    ));
}

#[test]
fn entering_beggar_registers_every_transition_for_intelligent_lacklandist_seekers() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, DetectableType, Posture};

    let mut engine = EngineInner::new();
    let beggar = engine.add_test_entity(make_test_pc(Posture::SimulatingBeggar));
    let eligible = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let low_iq = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let not_seeking = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let wrong_camp = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));

    for (id, iq, substate) in [
        // Hard difficulty doubles enemy IQ: the recorded boundary has a
        // base-IQ-15 seeker that Original admits at the effective threshold.
        (eligible, 15, Substate::SeekingSeekpointApproachingBeggar),
        (low_iq, 14, Substate::SeekingSeekpoint),
        (not_seeking, 100, Substate::DefaultOnPost),
        (wrong_camp, 100, Substate::SeekingSeekpoint),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("test observer must remain present")
        else {
            panic!("test observer must remain a soldier");
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test observer must retain enemy AI");
        ai.soldier_profile_iq = iq;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = substate;
    }

    crate::engine::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
        &mut engine.world.entities,
        &engine.mission_domain.diplomacy,
        beggar,
        crate::player_profile::DifficultyLevel::Hard,
    );
    // The shipped game permits duplicates. A repeated completed transition appends the
    // same Beggar detectable again.
    crate::engine::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
        &mut engine.world.entities,
        &engine.mission_domain.diplomacy,
        beggar,
        crate::player_profile::DifficultyLevel::Hard,
    );

    let beggar_idx = DetectableType::Beggar as usize;
    for (id, expected_count) in [
        (eligible, 2),
        (low_iq, 0),
        (not_seeking, 0),
        (wrong_camp, 0),
    ] {
        let list = &engine
            .get_entity(id)
            .and_then(Entity::npc_data)
            .expect("test observer must retain NPC data")
            .detectable_lists[beggar_idx];
        assert_eq!(list.len(), expected_count);
        assert!(
            list.iter()
                .all(|detectable| detectable.element == Some(beggar)
                    && detectable.detectable_type == DetectableType::Beggar),
            "unexpected beggar registration for {id:?}"
        );
    }
}
