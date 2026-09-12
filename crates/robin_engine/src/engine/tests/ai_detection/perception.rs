use super::*;

#[test]
fn primary_target_tracking_precedes_view_refresh() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let soldier_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let soldier_pos = MapPoint::new(100.0, 100.0);
    let target_pos = MapPoint::new(100.0, 200.0);

    if let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(soldier_id) {
        soldier.element.active = true;
        soldier.element.set_position_map(soldier_pos);
        soldier.element.set_direction_instantly(4);
        soldier.npc.direction_old = 4;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(pc_id.index()));
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingReactiontime;
    }
    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.active = true;
        pc.element.set_position_map(target_pos);
    }

    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(
        target_pos.x - soldier_pos.x,
        target_pos.y - soldier_pos.y,
    );
    let Entity::Soldier(soldier) = engine.get_entity(soldier_id).unwrap() else {
        panic!("test soldier changed entity kind");
    };
    assert_eq!(soldier.element.direction(), expected);
    assert_eq!(
        soldier.npc.direction_old, expected,
        "view refresh must observe the combat tracking direction in the same frame"
    );
}

#[test]
fn npc_hourglass_observes_exact_original_phase_order() {
    use super::super::tick::{NpcHourglassPhase as Phase, capture_npc_hourglass_phases};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let (_, phases) = capture_npc_hourglass_phases(|| {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
    });

    assert_eq!(
        phases,
        vec![
            Phase::SoldierPrelude,
            Phase::Patrol,
            Phase::BaseHuman,
            Phase::Broadcasts,
            Phase::View,
            Phase::Detection,
            Phase::Ambush,
            Phase::Busy,
            Phase::Ladder,
            Phase::LockGate,
            Phase::SixteenthFrame,
            Phase::NormalTimer,
            Phase::MacroTimer,
            Phase::QueuedStimuli,
        ]
    );
}

#[test]
fn periodic_bored_roll_reads_installed_order_after_detection_boundary() {
    use crate::element::InstalledActorOrder;
    use crate::order::OrderType;
    use crate::sim_rng::{RngSite, with_draw_trace};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // Model a synchronous detection decision replacing the order after the actor update
    // selected its tail order. The sequence manager deliberately has no selected
    // order: only ActorData::installed_order mirrors the original game's live order.
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(npc_id)
            .is_none()
    );
    engine
        .get_entity_mut(npc_id)
        .and_then(Entity::actor_data_mut)
        .expect("periodic owner has actor data")
        .installed_order = Some(InstalledActorOrder {
        order_id: std::num::NonZeroU32::new(1).unwrap(),
        order_type: OrderType::WaitingUprightBored,
    });
    // Register zero reaches the every-16-frame update at frame 100.
    engine.control.frame_counter = 100;

    let sim = crate::sim_rng::test_context();
    let (_, trace) = with_draw_trace(|| engine.tick_periodic_ai_for_npc(&sim, npc_id, &assets));

    assert!(
        trace.contains(&RngSite::VipIdleRemark),
        "the sixteenth-frame animation query must read the installed order at its own boundary"
    );
}

#[test]
fn periodic_enemy_post_refresh_reads_the_materialized_manager_queue_without_surfacing_completion() {
    use crate::ai::{AiContext, AiState, GotoFlags, Position, Substate};
    use crate::element::{Camp, Command, Entity};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();

    let mut run_case = |case: &str| {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let position = Position {
            x: 100.0,
            y: 100.0,
            sector: SectorHandle::new(0),
            ..Position::default()
        };
        let ctx = AiContext {
            position,
            self_animation: OrderType::WaitingAlerted,
            self_is_soldier: true,
            ..AiContext::test_fixture()
        };
        let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        soldier
            .element
            .set_position_map(MapPoint::new(position.x, position.y));
        soldier.element.set_sector(position.sector);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = owner.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToPhalanx;
        ai.base.stuck_counter = 2;
        // An enclosing Think owns this latch. The prefix boundary must not
        // manufacture its decision completion while materializing movement registration.
        ai.base.think_recursion_depth = 1;
        ai.base.completion_latch_inside_think = true;
        if case == "accepted" {
            ai.base.open_end_think_frames = 1;
            ai.base.engine_deferred_end_think_frames = 1;
            ai.base.engine_completion_verdict_resolved = false;
        }
        let destination = match case {
            "accepted" => Position {
                x: 140.0,
                ..position
            },
            "already" => position,
            "denied" => Position {
                x: 140.0,
                sector: None,
                ..position
            },
            _ => unreachable!(),
        };
        ai.base.go_to(destination, GotoFlags::RUN, &ctx);

        engine.finish_enemy_periodic_stuck_suffix_after_refresh(&sim, owner, &assets, 0, &ctx);
        let pending = engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::Null);
        let ai = engine.get_entity(owner).and_then(Entity::enemy_ai).unwrap();
        (
            pending,
            ai.base.stuck_counter,
            ai.base.completion_latch_inside_think,
            ai.base.already_on_point,
            ai.base.couldnt_reachpoint,
            ai.base.think_recursion_depth,
            ai.base.open_end_think_frames,
            ai.base.engine_deferred_end_think_frames,
            ai.base.engine_completion_verdict_resolved,
        )
    };

    assert_eq!(
        run_case("accepted"),
        (true, 0, true, false, false, 1, 1, 1, true),
        "accepted movement must register before the wildcard query, reset the watchdog, and retain the enclosing completion latch"
    );
    assert_eq!(
        run_case("already"),
        (false, 3, true, true, false, 1, 0, 0, false),
        "already-on-point movement leaves no manager element, so the selected Wait advances without surfacing completion"
    );
    assert_eq!(
        run_case("denied"),
        (false, 3, true, false, true, 1, 0, 0, false),
        "denied movement leaves no manager element, so the selected Wait advances without surfacing completion"
    );
}

#[test]
fn pc_noise_is_live_at_the_following_npc_slot_only() {
    use crate::element::{Camp, Detectable, DetectableType};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    fn observe(pc_first: bool) -> bool {
        let mut engine = EngineInner::new();
        let (pc, npc) = if pc_first {
            let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
            let npc = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            (pc, npc)
        } else {
            let npc = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
            (pc, npc)
        };
        // Static mission entities follow the Original's 31 hidden pre-level
        // creations. For the following NPC (slot 1), frame 1 opens the
        // three-frame hearing cadence: (1 + 31 + 1) % 3 == 0.
        engine.control.frame_counter = 1;
        let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
            panic!("noise PC changed kind")
        };
        pc_entity.element.active = true;
        pc_entity.element.set_position_map(MapPoint::new(55.0, 0.0));
        pc_entity.pc.life_points = 100;
        let Entity::Soldier(npc_entity) = engine.get_entity_mut(npc).expect("listener exists")
        else {
            panic!("listener changed kind")
        };
        npc_entity.element.active = true;
        npc_entity.element.set_position_map(MapPoint::new(0.0, 0.0));
        npc_entity.npc.life_points = 100;
        npc_entity
            .npc
            .ai_brain
            .enemy_mut()
            .expect("listener has enemy AI")
            .base
            .me = npc.index();
        npc_entity.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(pc),
            detectable_type: DetectableType::Enemy,
            ..Detectable::default()
        });

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(pc),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let positions = engine.boundary_positions_snapshot();
        crate::sim_rng::with_seed(0xA013_0015, |sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        });

        assert_eq!(
            engine
                .get_entity(pc)
                .and_then(Entity::actor_data)
                .expect("noise PC remains an actor")
                .last_noise_volume,
            70
        );
        engine
            .get_entity(npc)
            .and_then(Entity::npc_data)
            .expect("listener remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|detectable| detectable.element == Some(pc))
            .expect("listener retains the PC detectable")
            .heard_last_frame
    }

    assert!(
        observe(true),
        "a PC must publish its current noise before a following NPC detects"
    );
    assert!(
        !observe(false),
        "an NPC before the PC must retain prior-frame noise for this slot"
    );
}

#[test]
fn npc_post_detection_tail_is_wholly_creation_ordered_even_without_detection() {
    use super::super::ai::{
        NpcPostDetectionTailPhase as Tail, capture_npc_post_detection_tail_phases,
    };

    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let second = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [first, second] {
        let entity = engine.get_entity_mut(id).expect("tail owner exists");
        entity.element_data_mut().active = true;
        let npc = entity.npc_data_mut().expect("tail owner has NPC data");
        for list in &mut npc.detectable_lists {
            list.clear();
        }
        let ai = npc.ai_brain.base_mut().expect("tail owner has AI");
        ai.timer_is_running = false;
        ai.macro_timer_is_running = false;
    }

    let positions = engine.boundary_positions_snapshot();
    let (_, trace) = capture_npc_post_detection_tail_phases(|| {
        crate::sim_rng::with_seed(0xA013_7A11, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        })
    });

    let whole_tail = [
        Tail::Ambush,
        Tail::Deafness,
        Tail::Busy,
        Tail::Ladder,
        Tail::RandomSpeech,
        Tail::LockGate,
        Tail::SixteenthFrame,
        Tail::NormalTimer,
        Tail::MacroTimer,
        Tail::Emoticon,
        Tail::QueuedStimuli,
    ];
    let expected: Vec<_> = [first, second]
        .into_iter()
        .flat_map(|id| whole_tail.into_iter().map(move |phase| (id, phase)))
        .collect();
    assert_eq!(trace, expected);
}

#[test]
fn post_detection_tail_clears_only_unlocked_expired_emoticon() {
    use crate::ai::{AiLockFlags, EmoticonType};

    let mut engine = EngineInner::new();
    let unlocked = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let locked = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 200;
    for id in [unlocked, locked] {
        let ai = engine
            .get_entity_mut(id)
            .and_then(Entity::ai_controller_mut)
            .expect("emoticon owner has AI");
        ai.set_transient_emoticon(EmoticonType::QuestionMark, 1, 100);
    }
    engine
        .get_entity_mut(locked)
        .and_then(Entity::ai_controller_mut)
        .expect("locked emoticon owner has AI")
        .locks_flag_field = AiLockFlags::FREEZE;

    crate::sim_rng::with_seed(0xA013_EA10, |sim| {
        engine.tick_npc_post_detection_tail_for_npc(sim, unlocked, &assets);
        engine.tick_npc_post_detection_tail_for_npc(sim, locked, &assets);
    });
    let unlocked_ai = engine
        .get_entity(unlocked)
        .and_then(Entity::ai_controller)
        .expect("unlocked emoticon owner retains AI");
    assert_eq!(unlocked_ai.current_emoticon_type, EmoticonType::None);
    assert!(!unlocked_ai.emoticon_has_expiration_date);
    let locked_ai = engine
        .get_entity(locked)
        .and_then(Entity::ai_controller)
        .expect("locked emoticon owner retains AI");
    assert_eq!(locked_ai.current_emoticon_type, EmoticonType::QuestionMark);
    assert!(locked_ai.emoticon_has_expiration_date);
    assert_eq!(locked_ai.emoticon_expiration_date, 102);
}

#[test]
fn post_detection_tail_refreshes_deafness_off_acoustic_cadence() {
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 7;
    assert_ne!((engine.control.frame_counter + npc_id.index()) % 3, 0);
    let npc = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::npc_data_mut)
        .expect("deafness owner has NPC data");
    npc.old_cover_noise_deafness = 100;
    npc.old_cover_noise_deafness_frame_counter = 6;

    crate::sim_rng::with_seed(0xA013_DEAF, |sim| {
        engine.tick_npc_post_detection_tail_for_npc(sim, npc_id, &assets)
    });
    let npc = engine
        .get_entity(npc_id)
        .and_then(Entity::npc_data)
        .expect("deafness owner retains NPC data");
    assert_eq!(npc.old_cover_noise_deafness_frame_counter, 7);
    assert!(npc.old_cover_noise_deafness < 100);
}

#[test]
fn post_detection_tail_preserves_ladder_threshold_and_macro_stop_semantics() {
    use crate::ai::Substate;
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(npc_id));
    wait.state = SequenceState::InProgress;
    engine.orders.sequence_manager.launch_element(wait);
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).expect("ladder owner exists")
    else {
        panic!("ladder owner changed kind")
    };
    soldier.element.publish_order_posture(Posture::OnLadder);
    soldier.npc.stuck_on_ladder_emergency_counter = 25;
    let ai = soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("ladder owner has AI");
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;
    ai.current_substate = Substate::DefaultOnPost;

    engine.tick_npc_stuck_on_ladder_for_npc(sim, npc_id, &assets);
    assert_eq!(
        engine
            .get_entity(npc_id)
            .and_then(Entity::npc_data)
            .expect("ladder owner retains NPC data")
            .stuck_on_ladder_emergency_counter,
        0,
        "the 26th qualifying frame must trigger recovery and reset"
    );

    engine.tick_ai_macro_timer_for_npc(sim, npc_id, &assets);
    assert!(
        !engine
            .get_entity(npc_id)
            .and_then(Entity::ai_controller)
            .expect("macro owner retains AI")
            .macro_timer_is_running,
        "elapsed macro timers stop even outside DefaultInMacro"
    );
}

#[test]
fn normal_timer_does_not_turn_alerted_soldier_toward_primary_target() {
    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 100.0));
    let Entity::Soldier(soldier) = engine.get_entity_mut(npc_id).unwrap() else {
        panic!("timer owner changed kind")
    };
    soldier.element.set_position_map(MapPoint::ZERO);
    soldier.element.set_direction_instantly(5);
    soldier.npc.alerted = true;
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));
    ai.script_locked = true;
    ai.timer_is_running = true;
    ai.when_does_timer_ring = 0;
    ai.substate_at_last_timer_launch = ai.current_substate;

    engine.tick_ai_normal_timer_for_npc(sim, npc_id, &assets);

    let element = engine.get_entity(npc_id).unwrap().element_data();
    assert_eq!(element.direction(), 5);
    assert_eq!(
        element.sprite.position_iface.get_direction_goal().as_u8(),
        5,
        "original-game focus changes only the view cone; normal timer dispatch does not turn the actor"
    );
}

#[test]
fn civilian_macro_break_drains_missed_friend_detectables_immediately() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Detectable, DetectableType};

    let sim = &crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let friend_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("macro civilian exists")
    else {
        panic!("macro owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    civilian.npc.detectable_lists[DetectableType::MissedFriend as usize].push(Detectable {
        element: Some(friend_id),
        detectable_type: DetectableType::MissedFriend,
        ..Detectable::default()
    });
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("macro civilian has AI");
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    // CMD_FACE_TO without its u16 operand takes the common macro-interruption path,
    // which queues the MissedFriend deletions through set_checkpoint_charly(0).
    ai.macro_command = vec![3];
    ai.macro_command_offset = 0;
    ai.number_of_remaining_macro_bytes = 1;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 0;

    engine.tick_ai_macro_timer_for_npc(sim, civilian_id, &assets);

    let civilian = engine
        .get_entity(civilian_id)
        .and_then(Entity::npc_data)
        .expect("macro civilian retains NPC data");
    assert!(
        civilian.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
        "common macro completion deletes must be applied to civilian NpcData"
    );
    let ai = engine
        .get_entity(civilian_id)
        .and_then(Entity::ai_controller)
        .expect("macro civilian retains AI");
    assert!(ai.outbox.actor.deleted_detectable_types().is_empty());
}

#[test]
fn npc_body_broadcast_respects_swapped_creation_order_boundary() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, DetectableType, ElementData, ElementKind, Entity, Posture};

    #[derive(Debug, PartialEq)]
    struct Observation {
        body_before_observer: bool,
        body_slot: u32,
        observer_slot: u32,
        retained_stimuli: Vec<StimulusType>,
        body_detectables_after_tick: Vec<u32>,
        inform_flag_after_tick: bool,
    }

    fn observe(body_before_observer: bool) -> Observation {
        let mut engine = EngineInner::new();
        // Keep both NPCs away from the slot-zero special value used by a few
        // legacy AI handles, without introducing another detectable human.
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let body = make_test_ai_soldier(Camp::Lacklandists);
        let observer = make_test_ai_soldier(Camp::Lacklandists);
        let (body_id, observer_id) =
            crate::engine::test_support::actors::add_pair_in_creation_order(
                &mut engine,
                body,
                observer,
                body_before_observer,
            );

        for (id, x) in [(observer_id, 0.0), (body_id, 40.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order body test soldier exists")
            else {
                panic!("creation-order body test entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.direction_old = 4;
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 135;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order body test soldier has enemy AI");
            ai.base.me = id.index();
        }

        let Entity::Soldier(body) = engine
            .get_entity_mut(body_id)
            .expect("body exists before fixture completion")
        else {
            panic!("body changed kind")
        };
        body.human.unconscious = true;
        body.element.publish_order_posture(Posture::Lying);
        body.npc.inform_my_friends = true;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        // Isolate BODY detection and retain its raw stimulus so handler state
        // changes do not obscure whether this creation slot actually saw it.
        for id in [body_id, observer_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("body boundary soldier survives fixture completion")
            else {
                panic!("body boundary soldier changed kind after fixture")
            };
            for list in &mut soldier.npc.detectable_lists {
                list.clear();
            }
        }
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("body observer survives fixture completion")
        else {
            panic!("body observer changed kind after fixture")
        };
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("body observer retains enemy AI");
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingJustWatching;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::FREEZE;

        // The Body bucket refreshes strictly on the modulo-8 cadence of the
        // observer's modified frame (universal frame + creation order); open
        // that gate so the boundary question — did the broadcast land before
        // or after the observer's slot — is what decides the outcome.
        let observer_order = engine.world.original_creation_order(observer_id);
        engine.control.frame_counter = (8 - (observer_order % 8)) % 8;

        let positions_before_movement = engine.boundary_positions_snapshot();

        crate::sim_rng::with_seed(0xA013_0B0D, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(
                sim,
                &assets,
                &positions_before_movement,
            )
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("body observer remains an NPC");
        let observer_ai = observer
            .ai_brain
            .enemy()
            .expect("body observer remains enemy AI");
        let body = engine
            .get_entity(body_id)
            .and_then(Entity::npc_data)
            .expect("body remains an NPC");

        Observation {
            body_before_observer,
            body_slot: body_id.index(),
            observer_slot: observer_id.index(),
            retained_stimuli: observer_ai
                .base
                .stimulus_queue
                .iter()
                .map(|stimulus| stimulus.stimulus_type)
                .collect(),
            body_detectables_after_tick: observer.detectable_lists[DetectableType::Body as usize]
                .iter()
                .map(|detectable| {
                    detectable
                        .element
                        .expect("broadcast BODY detectable must retain its source")
                        .index()
                })
                .collect(),
            inform_flag_after_tick: body.inform_my_friends,
        }
    }

    assert_eq!(
        observe(true),
        Observation {
            body_before_observer: true,
            body_slot: 1,
            observer_slot: 2,
            retained_stimuli: vec![StimulusType::EventSeesBody],
            body_detectables_after_tick: vec![],
            inform_flag_after_tick: false,
        },
        "an earlier body must broadcast before the later observer detects and consume it"
    );
    assert_eq!(
        observe(false),
        Observation {
            body_before_observer: false,
            body_slot: 2,
            observer_slot: 1,
            retained_stimuli: vec![],
            body_detectables_after_tick: vec![2],
            inform_flag_after_tick: false,
        },
        "a later body may queue next-frame work but must not retroactively rescan an earlier observer"
    );
}

#[test]
fn inline_npc_recovery_precedes_simultaneous_body_inform_and_view() {
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let recovering_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(recovering) = engine.get_entity_mut(recovering_id).unwrap() else {
        panic!("recovering NPC changed kind")
    };
    recovering.element.active = true;
    recovering.npc.eye_status = EyeStatus::Closed;
    recovering.npc.inform_my_friends = true;
    let ai = recovering.npc.ai_brain.base_mut().unwrap();
    ai.outbox.recovery.inform_resurrection = true;
    ai.outbox.recovery.set_eye_status = Some(EyeStatus::LookForward);

    let Entity::Soldier(observer) = engine.get_entity_mut(observer_id).unwrap() else {
        panic!("observer changed kind")
    };
    observer.element.active = true;
    observer.npc.eye_status = EyeStatus::Closed;
    observer.npc.detectable_lists[DetectableType::Body as usize] = vec![Detectable {
        element: Some(recovering_id),
        detectable_type: DetectableType::Body,
        ..Detectable::default()
    }];

    engine.tick_ai_pending_resurrection_and_eyes_for_npc(recovering_id);

    let positions = engine.boundary_positions_snapshot();
    crate::sim_rng::with_seed(0x0A01_35A6, |sim| {
        engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
    });

    let recovering = engine
        .get_entity(recovering_id)
        .and_then(Entity::npc_data)
        .unwrap();
    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .unwrap();
    assert_eq!(recovering.eye_status, EyeStatus::LookForward);
    assert!(!recovering.inform_my_friends);
    assert!(
        !recovering
            .ai_brain
            .base()
            .unwrap()
            .outbox
            .recovery
            .inform_resurrection
    );
    assert_eq!(
        observer.detectable_lists[DetectableType::Body as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(recovering_id)],
        "recovery must delete the stale body first, then the simultaneous inform flag must re-add it"
    );
}

#[test]
#[should_panic(
    expected = "NPC 0 is missing its required AI controller while applying recovery state"
)]
fn npc_recovery_requires_an_ai_controller() {
    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    engine.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);
}

#[test]
fn subordinate_handles_shadow_locally_when_detected_chief_has_empty_patrol() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let source_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let subordinate_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(source_id, -20.0), (chief_id, 10.0), (subordinate_id, 0.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("patrol-dispatch test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.npc.view_radius = 400;
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    {
        let chief = engine
            .get_entity_mut(chief_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        chief.current_state = AiState::Default;
        chief.current_substate = Substate::DefaultOnPost;
        assert!(chief.patrol.is_empty());
    }
    {
        let subordinate = engine
            .get_entity_mut(subordinate_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        subordinate.current_state = AiState::Default;
        subordinate.current_substate = Substate::DefaultPatrolEnrouteWaiting;
        subordinate.patrol_chief = Some(chief_id);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::SendStimulus {
            target: subordinate_id.index(),
            stimulus_type: StimulusType::EventSeesShadow,
            info: StimulusInfo::Position(Position {
                x: 100.0,
                y: 0.0,
                ..Position::default()
            }),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    crate::sim_rng::with_seed(0xA013_2600, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let chief = engine.get_entity(chief_id).unwrap().enemy_ai().unwrap();
    assert_eq!(chief.base.current_state, AiState::Default);
    assert_eq!(chief.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(
        chief
            .last_stimulus_dispatched_to_patrol
            .as_ref()
            .map(|stimulus| stimulus.stimulus_type),
        Some(StimulusType::EventSeesShadow),
        "the empty chief still records the delegated stimulus before returning false"
    );
    let subordinate = engine
        .get_entity(subordinate_id)
        .unwrap()
        .enemy_ai()
        .unwrap();
    assert_eq!(subordinate.base.current_state, AiState::Default);
    assert_eq!(
        subordinate.base.current_substate,
        Substate::DefaultLookingShadow,
        "the subordinate must resume its local handler after the chief returns false"
    );
}

#[test]
fn enemy_tick_data_populates_live_patrol_chief_without_a_primary_target() {
    use crate::ai::AiState;
    use crate::coordinates::MapPoint;
    use crate::element::Camp;
    use crate::position_interface::SectorHandle;

    let mut engine = EngineInner::new();
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let minion_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    {
        let chief = engine.get_entity_mut(chief_id).unwrap();
        chief
            .element_data_mut()
            .set_position_map(MapPoint::new(1042.0, 1783.0));
        chief.element_data_mut().set_layer(2);
        chief.element_data_mut().set_sector(SectorHandle::new(61));
        chief.ai_controller_mut().unwrap().current_state = AiState::Wondering;
    }
    {
        let minion = engine.get_entity_mut(minion_id).unwrap();
        let ai = minion.ai_controller_mut().unwrap();
        ai.patrol_chief = Some(chief_id);
        ai.primary_target = None;
    }
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    crate::sim_rng::with_seed(0xA013_0469, |sim| {
        let tick = engine.build_npc_tick_data(sim, minion_id, &assets);
        assert_eq!(tick.patrol_chief_position.x, 1042.0);
        assert_eq!(tick.patrol_chief_position.y, 1783.0);
        assert_eq!(tick.patrol_chief_position.level, 2);
        assert_eq!(tick.patrol_chief_position.sector, SectorHandle::new(61));
        assert_eq!(tick.patrol_chief_state, AiState::Wondering);
    });
}

#[test]
fn sequence_completion_money_victim_scan_uses_live_off_detection_ko_registry() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::MapPoint;
    use crate::element::{Camp, Entity, Posture};
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let victim_far = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let inactive = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let victim_near = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let dead = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let conscious = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let wrong_camp = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    let victim_middle = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let ordinary_ko = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let stale_soldier_slot = engine.add_test_entity(make_test_civilian(Posture::Upright));

    let fixtures = [
        (owner_id, MapPoint::new(0.0, 0.0), true, 100, false, false),
        (victim_far, MapPoint::new(300.0, 0.0), true, 100, true, true),
        (inactive, MapPoint::new(5.0, 0.0), false, 100, true, true),
        (
            victim_near,
            MapPoint::new(100.0, 0.0),
            true,
            100,
            true,
            true,
        ),
        (dead, MapPoint::new(6.0, 0.0), true, 0, true, true),
        (conscious, MapPoint::new(7.0, 0.0), true, 100, false, true),
        (wrong_camp, MapPoint::new(8.0, 0.0), true, 100, true, true),
        (
            victim_middle,
            MapPoint::new(200.0, 0.0),
            true,
            100,
            true,
            true,
        ),
        (ordinary_ko, MapPoint::new(4.0, 0.0), true, 100, true, false),
    ];
    for (id, position, active, life_points, unconscious, money_fight_ko) in fixtures {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("fixture soldier exists")
        else {
            panic!("fixture changed entity kind")
        };
        soldier.element.active = active;
        soldier.element.publish_order_posture(if unconscious {
            Posture::Lying
        } else {
            Posture::Upright
        });
        soldier.element.set_position_map(position);
        soldier.npc.life_points = life_points;
        soldier.human.unconscious = unconscious;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("fixture soldier has Enemy AI");
        ai.base.me = id.index();
        ai.base.knocked_out_in_money_fight = money_fight_ko;
    }
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("owner has Enemy AI");
    owner.base.current_state = AiState::Wondering;
    owner.base.current_substate = Substate::WonderingWatchingForMoreMoney;
    // A sleeping AI substate is not itself unconscious: Original reads the
    // raw unconscious flag. Keep this raw-false control out of the list.
    engine
        .get_entity_mut(conscious)
        .and_then(Entity::enemy_ai_mut)
        .expect("sleeping-state control has Enemy AI")
        .base
        .current_substate = Substate::SleepingUnconscious;

    // Deliberately differ from entity-slot order. This is the authored
    // camp soldier order, including one stale handle whose slot is
    // now occupied by a civilian and must fail current typed validation.
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.ai.global.all_soldier_handles = std::sync::Arc::new(vec![
        victim_middle.index(),
        stale_soldier_slot.index(),
        owner_id.index(),
        inactive.index(),
        victim_far.index(),
        dead.index(),
        wrong_camp.index(),
        victim_near.index(),
        conscious.index(),
        ordinary_ko.index(),
    ]);
    engine.ai.standard_view_polygon_radius = 400;
    let scratch = engine.build_sim_scratch(&assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(owner_id).expect("owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.navigation.hiking_paths,
        &assets.navigation.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);

    assert_eq!(
        tick.camp_unconscious_soldiers
            .iter()
            .map(|candidate| (candidate.handle, candidate.knocked_out_in_money_fight))
            .collect::<Vec<_>>(),
        vec![
            (victim_middle.index(), true),
            (victim_far.index(), true),
            (victim_near.index(), true),
            (ordinary_ko.index(), false),
        ],
        "off-detection data keeps authored registry order and excludes stale-slot, inactive, dead, raw-conscious, and wrong-camp entries"
    );

    crate::sight_obstacle::begin_parity_visibility_capture();
    let (_, draws) = with_draw_trace(|| {
        engine.dispatch_think_with_drain(
            &sim,
            owner_id,
            &Stimulus::new(StimulusType::EventDone),
            &ctx,
            &tick,
            &assets,
        );
    });
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(queries.len(), 3);
    assert_eq!(
        queries
            .iter()
            .map(|query| query.destination[0])
            .collect::<Vec<_>>(),
        vec![200.0, 300.0, 100.0],
        "detection queries retain camp-registry order before distance sorting"
    );
    assert!(
        !draws.contains(&RngSite::MacroRand),
        "a live victim keeps sequence-completion EVENT_DONE out of ReturnToDuty: {draws:?}"
    );
    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("owner retains Enemy AI");
    assert_eq!(owner.base.current_state, AiState::Wondering);
    assert_eq!(
        owner.base.current_substate,
        Substate::WonderingApproachingToLoot
    );
    assert_eq!(
        owner.base.detected_body,
        Some(crate::ai::AiEntityHandle::new(victim_near.index()))
    );
}

#[test]
fn dispatch_ai_stimulus_intentionally_ignores_pcs() {
    use crate::ai::{Stimulus, StimulusType};
    use crate::element::{Entity, Posture};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_test_pc(Posture::Upright));

    engine.dispatch_ai_stimulus(pc_id, Stimulus::new(StimulusType::EventFitAgain));

    assert!(matches!(engine.get_entity(pc_id), Some(Entity::Pc(_))));
}

#[test]
fn wake_prefix_preserves_existing_stimulus_fifo() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let npc_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(npc_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap();
    ai.current_state = AiState::Default;
    ai.outbox.detection.stimuli = vec![
        Stimulus::new(StimulusType::EventLoseConsciousness),
        Stimulus::new(StimulusType::EventFitAgain),
        Stimulus::new(StimulusType::EventImpossible),
    ];

    let woke = crate::sim_rng::with_seed(0xA013_F1F0, |sim| {
        engine.dispatch_pending_fit_again_for_npc(sim, npc_id, &assets)
    });
    assert!(woke);
    let ai = engine
        .get_entity(npc_id)
        .and_then(Entity::ai_controller)
        .unwrap();
    assert_eq!(
        ai.outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventImpossible],
        "the older prefix through FITAGAIN must dispatch in FIFO order while only the suffix remains"
    );
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(
        ai.current_substate,
        Substate::SleepingAwakening,
        "LOSE_CONSCIOUSNESS must run before FITAGAIN; plucking FITAGAIN first would leave the NPC unconscious"
    );
    assert_eq!(
        ai.outbox.recovery.set_eye_status, None,
        "each synchronous Think in the restored FIFO prefix must commit its eye write inline"
    );
    assert_eq!(
        engine
            .get_entity(npc_id)
            .and_then(Entity::npc_data)
            .unwrap()
            .eye_status,
        EyeStatus::LookForward,
        "LOSE_CONSCIOUSNESS and the following FITAGAIN must publish their view-status writes in FIFO order"
    );
}

#[test]
fn npc_detection_observes_friend_state_at_creation_order_boundary() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::profiles::ProfileRank;

    fn observe(attacker_before_officer: bool) -> (AiState, AiState, Substate) {
        let mut engine = EngineInner::new();
        // Keep the relevant NPCs in slots 1/2 in both arrangements so the
        // swapped oracle is not confounded by a slot-zero special case.
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let attacker = make_test_ai_soldier(Camp::Lacklandists);
        let officer = make_test_ai_soldier(Camp::Lacklandists);
        let (attacker_id, officer_id) =
            crate::engine::test_support::actors::add_pair_in_creation_order(
                &mut engine,
                attacker,
                officer,
                attacker_before_officer,
            );
        let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

        for (id, x) in [(officer_id, 0.0), (attacker_id, 120.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order detection soldier exists")
            else {
                panic!("creation-order detection entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D { x, y: 0.0, z: 0.0 });
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.life_points = 100;
            soldier.npc.view_direction = [1.0, 0.0];
            soldier.npc.view_radius = 135;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order detection soldier has enemy AI");
            ai.base.me = id.index();
            ai.soldier_profile_rank = if id == officer_id {
                ProfileRank::Officer
            } else {
                ProfileRank::Soldier
            };
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("creation-order detection PC exists")
        else {
            panic!("creation-order detection target changed kind")
        };
        pc.element.active = true;
        pc.element.set_position(crate::coordinates::WorldPoint3D {
            x: 175.0,
            y: 0.0,
            z: 0.0,
        });
        pc.element.set_position_map(MapPoint::new(175.0, 0.0));
        pc.pc.life_points = 100;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        // Isolate the exact A retained-EVENT_VIEW tail → B-FRIEND edge after
        // fixture initialization has installed profiles and AI defaults.
        let Entity::Soldier(attacker) = engine
            .get_entity_mut(attacker_id)
            .expect("attacker exists before detection")
        else {
            panic!("attacker changed kind")
        };
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        attacker.npc.detectable_lists[DetectableType::Friend as usize].clear();
        attacker
            .npc
            .ai_brain
            .base_mut()
            .expect("attacker retains base AI")
            .stimulus_queue
            .push(Stimulus::with_human(StimulusType::EventView, pc_id.index()));

        let Entity::Soldier(officer) = engine
            .get_entity_mut(officer_id)
            .expect("officer exists before detection")
        else {
            panic!("officer changed kind")
        };
        officer.npc.detectable_lists[DetectableType::Friend as usize].clear();
        officer.npc.detectable_lists[DetectableType::Friend as usize].push(Detectable {
            element: Some(attacker_id),
            detectable_type: DetectableType::Friend,
            ..Detectable::default()
        });

        let positions = engine.boundary_positions_snapshot();
        crate::sim_rng::with_seed(0xA013, |sim| {
            engine.tick_enemy_ai_with_creation_ordered_prelude(sim, &assets, &positions)
        });

        let attacker_ai = engine
            .get_entity(attacker_id)
            .and_then(Entity::enemy_ai)
            .expect("attacker remains an enemy AI");
        let officer_ai = engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("officer remains an enemy AI");
        (
            attacker_ai.base.current_state,
            officer_ai.base.current_state,
            officer_ai.base.current_substate,
        )
    }

    let attacker_first = observe(true);
    assert_eq!(attacker_first.0, AiState::Attacking);
    assert_eq!(
        attacker_first.1,
        AiState::Default,
        "later officer must see that the earlier EVENT_VIEW made its friend unable to help"
    );
    assert_ne!(attacker_first.2, Substate::SeekingOfficerCallSoldier);
    // The officer already faces the helpful soldier, so the Face inside the
    // FRIEND sighting is a no-op and the Think tail posts EVENT_DONE
    // synchronously: the soldier is hailed in the same slot and the
    // officer ends the tick already waiting for him.
    assert_eq!(
        observe(false),
        (
            AiState::Attacking,
            AiState::Seeking,
            Substate::SeekingOfficerWaitForSoldier,
        ),
        "earlier officer must see the still-helpful soldier before that soldier handles EVENT_VIEW"
    );
}

#[test]
fn npc_hearing_thinks_before_same_slot_optical_detection() {
    use crate::ai::AiState;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Keep the NPC out of legacy slot zero and choose the frame so its
    // `(frame + creation_order) % DETECTION_FREQUENCY_SOUNDS` gate is open.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    // Slot 1 has Original creation order 32 after the hidden pre-level
    // prefix, so frame 1 opens its three-frame hearing cadence.
    engine.control.frame_counter = 1;

    let soldier_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let stale_pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("hearing-order soldier exists")
    else {
        panic!("hearing-order entity changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("hearing-order soldier has enemy AI");
    ai.base.me = soldier_id.index();

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("hearing-order PC exists")
    else {
        panic!("hearing-order target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position(crate::coordinates::WorldPoint3D {
        x: 55.0,
        y: 0.0,
        z: 0.0,
    });
    pc.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Pc(stale_pc) = engine
        .get_entity_mut(stale_pc_id)
        .expect("stale hearing-order PC exists")
    else {
        panic!("stale hearing-order target changed kind")
    };
    stale_pc.element.active = false;
    stale_pc.pc.life_points = 0;

    // RunningUpright on ground produces a 70-volume TAPTAPTAP. At 55 units
    // this becomes the original's 15-volume subjective noise, while keeping
    // EVENT_VIEW outside the unrelated close-combat branch (< 50 units).
    // Install it as the production snapshot builder's current animation
    // instead of injecting a synthetic noise into the detection helper.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(pc_id),
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

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("soldier exists before hearing-order detection")
    else {
        panic!("hearing-order soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 0;
    // Acoustics runs before optical detectable cleanup. A just-dead PC can
    // therefore still occupy an earlier enemy-list slot without appearing in
    // the alive-only world snapshot; it must not block the later audible PC.
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(stale_pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        // Keep the oracle about HEAR → VIEW rather than predetection shadow.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0EAD, |sim| engine.tick_enemy_ai(sim, &assets));

    assert_eq!(
        engine
            .get_entity(pc_id)
            .and_then(Entity::actor_data)
            .expect("hearing-order PC remains an actor")
            .last_noise_volume,
        70,
        "production PC snapshot must derive the expected running noise"
    );
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("hearing-order soldier remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize][0]
            .heard_last_frame,
        "production acoustic pass must reach the hearing-update latch"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("soldier remains an enemy AI");
    assert_eq!(
        ai.base.current_state,
        AiState::Attacking,
        "synchronous EVENT_HEAR must make optical detection instant in the same detection refresh"
    );
}

#[test]
fn detection_tick_preserves_authoritative_enemy_membership() {
    use crate::element::{Camp, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("membership observer exists")
    else {
        panic!("membership observer changed kind")
    };
    observer.element.active = true;
    observer.npc.life_points = 100;

    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("untracked PC exists") else {
        panic!("untracked target changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("membership observer exists after fixture")
    else {
        panic!("membership observer changed kind after fixture")
    };
    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();

    // Slot zero has Original creation order 31. Frame two opens the modulo-3
    // acoustic gate as well as running the optical pass, so this one tick
    // exercises both places that formerly reconciled every missing PC.
    engine.control.frame_counter = 2;
    crate::sim_rng::with_seed(0xA013_0EAE, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("membership observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize].is_empty(),
        "detection refresh must only iterate serialized/explicitly-added detectables; it must not synthesize a missing PC"
    );
}

#[test]
fn lackland_detection_scans_and_retains_full_fifo_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{
        Camp, Detectable, DetectableType, ElementBonus, ElementData, ElementKind, Entity,
    };
    use crate::element_kinds::ObjectType;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    // The observer's modified frame is universal frame + its creation order
    // (31 hidden pre-mission elements, then the Target, so the observer sits
    // at 32). Every strict per-bucket cadence in this oracle — hearing (3),
    // Body (8), Object (4), Enemy-PC (2) — must be open in the same tick,
    // so pick a frame with 16 + 32 = 48 ≡ 0 mod lcm(3, 8, 4, 2) = 24.
    engine.control.frame_counter = 16;

    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_visible_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let lost_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let last_visible_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));
    let object_id = engine.add_test_entity(Entity::Bonus(ElementBonus {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectBonus;
            initial_element.active = true;
            initial_element
        },
        object: crate::element::ObjectData {
            object_type: ObjectType::Coin,
            ..crate::element::ObjectData::default()
        },
    }));
    let friend_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists")
    else {
        panic!("locked detection observer changed kind")
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

    for (id, x, life_points) in [
        (first_visible_id, 55.0, 100),
        (lost_id, -200.0, 100),
        (last_visible_id, 80.0, 100),
        (body_id, 100.0, 0),
    ] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(id)
            .expect("locked detection PC exists")
        else {
            panic!("locked detection PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = life_points;
    }

    let Entity::Bonus(object) = engine
        .get_entity_mut(object_id)
        .expect("locked detection object exists")
    else {
        panic!("locked detection object changed kind")
    };
    object
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0));
    object.element.set_position_map(MapPoint::new(100.0, 0.0));

    let Entity::Soldier(friend) = engine
        .get_entity_mut(friend_id)
        .expect("locked observer's friend exists")
    else {
        panic!("locked observer's friend changed kind")
    };
    friend.element.active = true;
    friend
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-20.0, 20.0, 0.0));
    friend.element.set_position_map(MapPoint::new(-20.0, 20.0));
    friend.npc.life_points = 100;
    friend.npc.eye_status = crate::element::EyeStatus::Closed;

    // RunningUpright produces the production 70-volume TAPTAPTAP used by
    // RefreshDetection's acoustic pass; the frame chosen above keeps the
    // observer's three-frame hearing cadence open.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(first_visible_id),
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

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists after fixture")
    else {
        panic!("locked detection observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("locked detection observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::FREEZE;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detectable_lists[DetectableType::Body as usize].clear();
    observer.npc.detectable_lists[DetectableType::Object as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Body as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Object as usize] = 999;
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        // Keep this oracle's shadow prefix confined to the Enemy bucket.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    observer.npc.detectable_lists[DetectableType::Object as usize].push(Detectable {
        element: Some(object_id),
        detectable_type: DetectableType::Object,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B22, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("locked detection observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| (
                det.heard_last_frame,
                det.shadow_seen_last_frame,
                det.seen_last_frame,
            ))
            .collect::<Vec<_>>(),
        vec![
            (true, true, true),
            (false, false, false),
            (false, true, true)
        ],
        "AI lock must not suppress acoustic, predetection, or optical latch updates"
    );
    assert_eq!(
        observer.detection_suspects[DetectableType::Enemy as usize],
        0,
        "locked Enemy detection must still commit and reset suspects"
    );
    assert!(
        observer.detectable_lists[DetectableType::Body as usize].is_empty(),
        "locked non-Enemy buckets must still commit one-shot detectables"
    );
    assert!(
        observer.detectable_lists[DetectableType::Object as usize].is_empty(),
        "locked Object detection must still commit its one-shot detectable"
    );

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("locked detection observer retains enemy AI");
    assert_eq!(
        (ai.base.current_state, ai.base.current_substate),
        (AiState::Default, Substate::DefaultOnPost)
    );
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(body_id.index()))
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventHear,
            StimulusType::EventSeesShadow,
            StimulusType::EventSeesShadow,
            StimulusType::EventView,
            StimulusType::EventOutOfView,
            StimulusType::EventView,
            StimulusType::EventSeesBody,
            StimulusType::EventSeesObject,
        ],
        "decision entry must retain the complete HEAR then optical FIFO under AI lock"
    );
    assert!(matches!(
        ai.base.stimulus_queue[0].info,
        StimulusInfo::Noise(_)
    ));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .filter_map(|stimulus| match stimulus.info {
                StimulusInfo::Human(target) => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            crate::ai::AiEntityHandle::new(first_visible_id.index()),
            crate::ai::AiEntityHandle::new(lost_id.index()),
            crate::ai::AiEntityHandle::new(last_visible_id.index()),
            crate::ai::AiEntityHandle::new(body_id.index()),
        ]
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .last()
            .expect("Object event closes the retained detection FIFO")
            .info,
        StimulusInfo::Object(crate::ai::AiEntityHandle::new(object_id.index())),
        "EVENT_SEES_OBJECT must retain an object payload, not impersonate a human"
    );
    assert_eq!(
        engine
            .get_entity(friend_id)
            .and_then(Entity::npc_data)
            .expect("locked observer's friend remains an NPC")
            .ai_state(),
        AiState::Default,
        "a retained VIEW must not leak through the later out-of-band ally alert"
    );
    assert!(
        !engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("locked detection observer remains an NPC")
            .alerted,
        "AILOCK_FREEZE must retain VIEW without pre-alerting its observer"
    );

    // The original game's global AI freeze is a separate mode: the
    // next detection refresh still scans and commits its latch, but AI admission
    // discards the resulting VIEW instead of retaining it.
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("static-freeze detection observer exists")
    else {
        panic!("static-freeze detection observer changed kind")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("static-freeze detection observer retains enemy AI");
    ai.base.locks_flag_field = AiLockFlags::empty();
    ai.base.stimulus_queue.clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame = false;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].shadow_seen_last_frame = true;

    engine.ai.global.freeze = true;
    crate::sim_rng::with_seed(0xA013_0B24, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("static-freeze detection observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "static AI freeze must not suppress detection-refresh latch commits"
    );
    let ai = observer
        .ai_brain
        .enemy()
        .expect("static-freeze detection observer retains enemy AI");
    assert!(
        ai.base.stimulus_queue.is_empty(),
        "static AI freeze must discard detection stimuli"
    );
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!observer.alerted);
}

#[test]
fn retained_detection_view_rebuilds_the_live_enemy_scan_on_replay() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let rising_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let already_seen_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists")
    else {
        panic!("queued replay observer changed kind")
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

    for (id, x) in [(rising_id, 80.0), (already_seen_id, 120.0)] {
        let Entity::Pc(pc) = engine.get_entity_mut(id).expect("queued replay PC exists") else {
            panic!("queued replay PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists after fixture")
    else {
        panic!("queued replay observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("queued replay observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.list_them.clear();

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    for (target_id, seen_last_frame) in [(rising_id, false), (already_seen_id, true)] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B23, |sim| engine.tick_enemy_ai(sim, &assets));
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI");
    assert_eq!(ai.base.stimulus_queue.len(), 1);
    assert_eq!(ai.base.current_state, AiState::Default);

    engine
        .get_entity_mut(observer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("queued replay observer retains controller")
        .locks_flag_field = AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(sim, &assets);

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI after replay");
    assert!(ai.base.stimulus_queue.is_empty());
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(
        ai.base.primary_target,
        Some(crate::ai::AiEntityHandle::new(rising_id.index()))
    );
    assert_eq!(
        ai.list_them,
        vec![rising_id.index(), already_seen_id.index()],
        "retained VIEW replay must rebuild all currently latched enemies, not seed only its payload"
    );
    assert!(
        engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("queued replay observer remains an NPC after replay")
            .alerted,
        "accepted retained VIEW must set the persistent alert marker at dispatch time"
    );
}

#[test]
fn npc_out_of_view_precedes_same_slot_body_fifo() {
    use crate::ai::{AiState, Position, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));

    let soldier_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists")
    else {
        panic!("out-of-view observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("out-of-view soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingObserve;
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(lost_pc_id.index()));
    ai.base.seek_position = Position {
        x: -200.0,
        y: 0.0,
        ..Position::default()
    };
    ai.current_task_priority = task_priority::ENEMY;

    let Entity::Pc(lost_pc) = engine.get_entity_mut(lost_pc_id).expect("lost PC exists") else {
        panic!("lost target changed kind")
    };
    lost_pc.element.active = true;
    lost_pc
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-200.0, 0.0, 0.0));
    lost_pc.element.set_position_map(MapPoint::new(-200.0, 0.0));
    lost_pc.pc.life_points = 100;

    let Entity::Pc(body) = engine.get_entity_mut(body_id).expect("body PC exists") else {
        panic!("body target changed kind")
    };
    body.element.active = true;
    body.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    body.element.set_position_map(MapPoint::new(80.0, 0.0));
    body.pc.life_points = 0;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the living PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists before detection")
    else {
        panic!("out-of-view soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Body as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(lost_pc_id),
        detectable_type: DetectableType::Enemy,
        seen_last_frame: true,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0x0A01_30A7, |sim| engine.tick_enemy_ai(sim, &assets));

    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::npc_data)
        .expect("out-of-view soldier remains an NPC");
    assert!(
        !soldier.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "lost enemy must clear its seen latch"
    );
    assert!(
        soldier.detectable_lists[DetectableType::Body as usize].is_empty(),
        "visible body must commit and leave its one-shot detectable list"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("out-of-view soldier retains enemy AI");
    assert_eq!(
        ai.base.detected_body,
        Some(crate::ai::AiEntityHandle::new(body_id.index())),
        "OUTOFVIEW must enter Seeking before the later BODY stimulus is handled"
    );
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingBodyReactiontime);
}

#[test]
fn npc_detection_queues_every_rising_enemy_in_detectable_order() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(far_first: bool) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        let mut engine = EngineInner::new();
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let soldier_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        // A learned-through disguise remains in the Enemy bucket and must
        // emit ordinary EVENT_VIEW, never EVENT_SEES_BEGGAR.
        let far_pc_id =
            engine.add_test_entity(make_test_pc(crate::element::Posture::SimulatingBeggar));
        let near_pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists")
        else {
            panic!("multi-view observer changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.life_points = 100;
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 300;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = crate::element::EyeStatus::Stare;

        for (pc_id, x) in [(far_pc_id, 120.0), (near_pc_id, 80.0)] {
            let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("multi-view PC exists") else {
                panic!("multi-view target changed kind")
            };
            pc.element.active = true;
            pc.element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            pc.element.set_position_map(MapPoint::new(x, 0.0));
            pc.pc.life_points = 100;
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists before detection")
        else {
            panic!("multi-view observer changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("multi-view soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.got_the_beggar_trick = true;
        ai.list_them.clear();

        let ordered_targets = if far_first {
            [far_pc_id, near_pc_id]
        } else {
            [near_pc_id, far_pc_id]
        };
        let expected_order: Vec<u32> = ordered_targets.iter().map(|id| id.index()).collect();
        soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        for target_id in ordered_targets {
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            });
        }

        crate::sim_rng::with_seed(0xA013_0B1E, |sim| engine.tick_enemy_ai(sim, &assets));

        let soldier = engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("multi-view soldier remains an NPC");
        let latches = soldier.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect();
        assert_eq!(
            soldier.follow_target,
            Some(ordered_targets[0]),
            "the first accepted VIEW must retain focus after the later FIFO entry"
        );
        let ai = engine
            .get_entity(soldier_id)
            .and_then(Entity::enemy_ai)
            .expect("multi-view soldier retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(
            ai.base.primary_target,
            Some(crate::ai::AiEntityHandle::new(expected_order[0])),
            "the first detectable's VIEW must win even when a later target is nearer"
        );
        assert_eq!(
            ai.base.last_stimulus_actor,
            Some(crate::ai::AiEntityHandle::new(expected_order[1])),
            "the second VIEW must run through its own complete Think boundary"
        );
        (ai.list_them.clone(), latches, expected_order)
    }

    let (far_then_near, far_then_near_latches, far_then_near_expected) = observe(true);
    let (near_then_far, near_then_far_latches, near_then_far_expected) = observe(false);

    assert_eq!(far_then_near_latches, vec![true, true]);
    assert_eq!(near_then_far_latches, vec![true, true]);
    assert_eq!(far_then_near, far_then_near_expected);
    assert_eq!(near_then_far, near_then_far_expected);
}

#[test]
fn royalist_detection_alert_does_not_bypass_strict_cadence() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(source_before_listener: bool) -> (bool, AiState, bool) {
        let mut engine = EngineInner::new();
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let source = make_test_ai_soldier(Camp::Royalists);
        let listener = make_test_ai_soldier(Camp::Royalists);
        let (source_id, listener_id) =
            crate::engine::test_support::actors::add_pair_in_creation_order(
                &mut engine,
                source,
                listener,
                source_before_listener,
            );
        let target_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(source_id, 0.0), (listener_id, 20.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist ordering soldier exists")
            else {
                panic!("Royalist ordering actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 200;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            if id == target_id {
                soldier.element.blipped = true;
            }
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        for id in [source_id, listener_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist observer exists after fixture")
            else {
                panic!("Royalist observer changed kind after fixture")
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("Royalist observer has enemy AI");
            ai.base.me = id.index();
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
            ai.current_task_priority = task_priority::NONE;
            ai.base.primary_target = None;
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                ..Detectable::default()
            });
        }

        let Entity::Soldier(source) = engine
            .get_entity_mut(source_id)
            .expect("source Royalist exists")
        else {
            panic!("source Royalist changed kind")
        };
        source.element.set_direction_instantly(4);
        source.npc.view_direction = [1.0, 0.0];
        source.npc.eye_status = crate::element::EyeStatus::Stare;

        let Entity::Soldier(listener) = engine
            .get_entity_mut(listener_id)
            .expect("listener Royalist exists")
        else {
            panic!("listener Royalist changed kind")
        };
        listener.element.set_direction_instantly(4);
        listener.npc.view_direction = [1.0, 0.0];
        listener.npc.eye_status = crate::element::EyeStatus::LookForward;

        const ORIGINAL_PRE_LEVEL_CREATIONS: u32 = 31;
        let source_creation_order = source_id.index() + ORIGINAL_PRE_LEVEL_CREATIONS;
        let listener_creation_order = listener_id.index() + ORIGINAL_PRE_LEVEL_CREATIONS;
        engine.control.frame_counter = (crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC
            - source_creation_order % crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC)
            % crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC;
        assert!(
            (engine.control.frame_counter + source_creation_order)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "source fixture must start on an open Royalist NPC detection gate"
        );
        assert!(
            !(engine.control.frame_counter + listener_creation_order)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "listener fixture must start on a closed Royalist NPC detection gate"
        );
        crate::sim_rng::with_seed(0xA013_0B20, |sim| engine.tick_enemy_ai(sim, &assets));
        assert!(
            !engine
                .get_entity(target_id)
                .expect("Royalist target remains present")
                .element_data()
                .blipped,
            "Royalist detection must reveal its blipped NPC target at the detecting slot"
        );

        let source = engine
            .get_entity(source_id)
            .and_then(Entity::npc_data)
            .expect("source Royalist remains an NPC");
        let source_latch = source.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("source retains target detectable")
            .seen_last_frame;
        let source_ai = engine
            .get_entity(source_id)
            .and_then(Entity::enemy_ai)
            .expect("source Royalist retains enemy AI");
        assert_eq!(
            (source_latch, source_ai.base.current_state),
            (true, AiState::Attacking),
            "source Royalist must detect before its alert can test creation ordering"
        );

        let listener = engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener Royalist remains an NPC");
        let latch = listener.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("listener retains target detectable")
            .seen_last_frame;
        let ai = engine
            .get_entity(listener_id)
            .and_then(Entity::enemy_ai)
            .expect("listener Royalist retains enemy AI");
        (
            latch,
            ai.base.current_state,
            ai.base.primary_target == Some(crate::ai::AiEntityHandle::new(target_id.index())),
        )
    }

    let source_first = observe(true);
    assert_eq!(
        source_first,
        (false, AiState::Wondering, false),
        "a first Royalist's alert must not bypass the later Royalist's closed modulo-16 gate"
    );

    let listener_first = observe(false);
    assert_eq!(
        listener_first,
        (false, AiState::Wondering, false),
        "an earlier closed listener slot must not retroactively rescan after the source alerts it"
    );
}

#[test]
fn royalist_detection_retains_every_ordered_view_edge_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 1 opens its strict
    // modulo-16 Royalist NPC cadence.
    engine.control.frame_counter = 1;
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    let first_visible_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let last_visible_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

    for (id, x) in [
        (observer_id, 0.0),
        (first_visible_id, 80.0),
        (lost_id, 100.0),
        (last_visible_id, 120.0),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("Royalist multi-edge soldier exists")
        else {
            panic!("Royalist multi-edge actor changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.element.blipped = id != observer_id;
    }

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists")
    else {
        panic!("Royalist multi-edge observer changed kind")
    };
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(lost) = engine
        .get_entity_mut(lost_id)
        .expect("lost Royalist target exists after fixture")
    else {
        panic!("lost Royalist target changed kind after fixture")
    };
    // The original game removes dead enemies during detectable cleanup, not inactive living
    // ones. An inactive outdoor target remains in the list and emits the
    // falling OUTOFVIEW edge.
    lost.element.active = false;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists after fixture")
    else {
        panic!("Royalist multi-edge observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("Royalist multi-edge observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B21, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("Royalist multi-edge observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect::<Vec<_>>(),
        vec![true, false, true],
        "detection must settle every Royalist enemy latch before AI processing"
    );
    assert!(
        !engine
            .get_entity(first_visible_id)
            .expect("first visible Royalist target remains present")
            .element_data()
            .blipped
            && !engine
                .get_entity(last_visible_id)
                .expect("last visible Royalist target remains present")
                .element_data()
                .blipped,
        "every rising Royalist Enemy edge must reveal its target before Think"
    );
    assert!(
        engine
            .get_entity(lost_id)
            .expect("lost Royalist target remains present")
            .element_data()
            .blipped,
        "a falling Royalist Enemy edge must not reveal its target"
    );
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("Royalist multi-edge observer retains enemy AI");
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(last_visible_id.index()))
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("Royalist Enemy edge lost its human payload")
                };
                (stimulus.stimulus_type, target)
            })
            .collect::<Vec<_>>(),
        vec![
            (
                StimulusType::EventView,
                crate::ai::AiEntityHandle::new(first_visible_id.index()),
            ),
            (
                StimulusType::EventOutOfView,
                crate::ai::AiEntityHandle::new(lost_id.index()),
            ),
            (
                StimulusType::EventView,
                crate::ai::AiEntityHandle::new(last_visible_id.index()),
            ),
        ],
        "Royalist detection must retain interleaved edges in detectable-list order"
    );
}

#[test]
fn royalist_enemy_cadence_stays_strict_when_staring_following_or_alerted() {
    use crate::ai::{AiLockFlags, AlertLevel};
    use crate::element::{Camp, Detectable, DetectableType, Entity, EyeStatus};

    for (eye_status, alert_status) in [
        (EyeStatus::Stare, AlertLevel::Green),
        (EyeStatus::Follow, AlertLevel::Green),
        (EyeStatus::LookForward, AlertLevel::Red),
    ] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let target_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(observer_id, 0.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("strict-cadence soldier exists")
            else {
                panic!("strict-cadence actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("strict-cadence observer exists after fixture")
        else {
            panic!("strict-cadence observer changed kind after fixture")
        };
        observer.element.set_direction_instantly(4);
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 200;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = eye_status;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("strict-cadence observer has EnemyAi");
        ai.base.current_music_alert_status = alert_status;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_now: true,
            seen_last_frame: true,
            last_visibility: 0.25,
            ..Detectable::default()
        }];

        // Slot 0 has Original creation order 31; frame 2 is a closed strict
        // modulo-16 cadence frame.
        engine.control.frame_counter = 2;
        assert!(
            !(engine.control.frame_counter + observer_id.index() + 31)
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC)
        );
        crate::sim_rng::with_seed(0xA013_1600 + eye_status as u64, |sim| {
            engine.tick_enemy_ai(sim, &assets)
        });

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("strict-cadence observer retains NPC state");
        let detectable = &observer.detectable_lists[DetectableType::Enemy as usize][0];
        assert_eq!(
            (
                detectable.seen_now,
                detectable.seen_last_frame,
                detectable.last_visibility
            ),
            (true, true, 0.25),
            "Royalist {:?}/{:?} must reuse the cached sample without recomputing on a closed modulo-16 gate",
            eye_status,
            alert_status
        );
        assert!(
            observer
                .ai_brain
                .base()
                .expect("strict-cadence observer retains AI state")
                .stimulus_queue
                .is_empty()
        );
    }
}

#[test]
fn royalist_civilian_enemy_list_accepts_pc_but_not_lacklandist_soldier() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{AiBrain, Camp, DetectableType, Entity};

    let mut engine = EngineInner::new();
    // Slot 0 has Original creation order 31; frame 1 opens the Royalist
    // modulo-16 Enemy cadence.
    engine.control.frame_counter = 1;
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let lacklandist_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("Royalist civilian exists")
    else {
        panic!("Royalist civilian changed kind")
    };
    civilian.element.active = true;
    civilian.civilian.cached_camp = Camp::Royalists;
    civilian.npc.life_points = 100;
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        civilian_id.index(),
    )));
    civilian.element.set_direction_instantly(4);
    civilian.npc.view_direction = [1.0, 0.0];
    civilian.npc.view_radius = 200;
    civilian.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;

    for (id, x) in [(pc_id, 80.0), (lacklandist_id, 100.0)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("Royalist-civilian target exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
        match entity {
            Entity::Pc(pc) => pc.pc.life_points = 100,
            Entity::Soldier(soldier) => soldier.npc.life_points = 100,
            _ => panic!("Royalist-civilian target changed kind"),
        }
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    // Enemy membership is established once by AI initialization's insertion policy
    // (the tick preserves it, never rebuilds it): a Royalist civilian keeps
    // PCs and rejects soldiers of either camp.
    let init_detectables = crate::engine::ai::build_detectable_enemies_for(
        Camp::Royalists,
        true,
        civilian_id,
        &crate::engine::ai::build_potential_detectables(&engine),
    );
    assert_eq!(
        init_detectables
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(pc_id)],
        "Royalist civilian detectable insertion accepts PCs only"
    );
    let civilian = engine
        .get_entity_mut(civilian_id)
        .and_then(Entity::npc_data_mut)
        .expect("Royalist civilian retains NPC state");
    civilian.detectable_lists[DetectableType::Enemy as usize] = init_detectables;
    civilian.detection_suspects[DetectableType::Enemy as usize] = 999;
    civilian
        .ai_brain
        .base_mut()
        .expect("Royalist civilian retains FriendlyAi")
        .locks_flag_field = AiLockFlags::BUSY;

    crate::sim_rng::with_seed(0xA013_C1A1, |sim| engine.tick_enemy_ai(sim, &assets));

    let civilian = engine
        .get_entity(civilian_id)
        .and_then(Entity::npc_data)
        .expect("Royalist civilian survives detection");
    assert_eq!(
        civilian.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(pc_id)],
        "Royalist civilian detectable insertion accepts PCs only"
    );
    let ai = civilian
        .ai_brain
        .base()
        .expect("Royalist civilian retains FriendlyAi after detection");
    assert_eq!(
        ai.stimulus_queue
            .iter()
            .filter(|stimulus| stimulus.stimulus_type == StimulusType::EventView)
            .map(|stimulus| stimulus.info)
            .collect::<Vec<_>>(),
        vec![StimulusInfo::Human(crate::ai::AiEntityHandle::new(
            pc_id.index()
        ))]
    );
}

#[test]
fn enemy_outer_box_rejection_preserves_shadow_latch_but_entered_invisible_clears_it() {
    use crate::element::{DetectableType, Entity};

    fn shadow_latch_after_scan(target_x: f32) -> (bool, bool, f32) {
        let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("outer-box target exists")
        else {
            panic!("outer-box target changed kind")
        };
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(target_x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(target_x, 0.0));

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("outer-box observer exists")
        else {
            panic!("outer-box observer changed kind")
        };
        let enemies = &mut observer.npc.detectable_lists[DetectableType::Enemy as usize];
        enemies.retain(|detectable| detectable.element == Some(pc_id));
        let detectable = enemies
            .first_mut()
            .expect("outer-box fixture retains its PC detectable");
        detectable.shadow_seen_last_frame = true;
        detectable.seen_now = true;
        detectable.last_visibility = 0.0;
        observer.npc.detection_suspects[DetectableType::Enemy as usize] =
            crate::ai_vision::SHADOW_DETECTION_THRESHOLD as u16;

        crate::sim_rng::with_seed(0xA013_0B0E, |sim| engine.tick_enemy_ai(sim, &assets));

        let detectable = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("outer-box observer retains NPC state")
            .detectable_lists[DetectableType::Enemy as usize]
            .first()
            .expect("outer-box observer retains PC detectable");
        (
            detectable.shadow_seen_last_frame,
            detectable.seen_now,
            detectable.last_visibility,
        )
    }

    let outside = shadow_latch_after_scan(400.0);
    assert_eq!(
        outside,
        (true, false, 0.0),
        "the rejected-target branch clears current visibility without predetection"
    );

    let entered_but_behind = shadow_latch_after_scan(-80.0);
    assert_eq!(
        entered_but_behind,
        (false, false, 0.0),
        "an entered target with zero sharpness must run predetection and clear the old latch"
    );
}

#[test]
fn lacklandist_mixed_pc_soldier_enemy_fifo_follows_detectable_order() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::Entity;

    for pc_first in [true, false] {
        let (mut engine, assets, observer_id, pc_id, royalist_id) =
            mixed_enemy_fifo_fixture(pc_first);
        crate::sim_rng::with_seed(0xA013_F1F0, |sim| engine.tick_enemy_ai(sim, &assets));

        let ai = engine
            .get_entity(observer_id)
            .and_then(Entity::ai_controller)
            .expect("mixed-fifo observer retains its controller");
        let actual = ai
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                assert_eq!(stimulus.stimulus_type, StimulusType::EventView);
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("mixed Enemy VIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if pc_first {
            vec![
                crate::ai::AiEntityHandle::new(pc_id.index()),
                crate::ai::AiEntityHandle::new(royalist_id.index()),
            ]
        } else {
            vec![
                crate::ai::AiEntityHandle::new(royalist_id.index()),
                crate::ai::AiEntityHandle::new(pc_id.index()),
            ]
        };
        assert_eq!(
            actual, expected,
            "one detection pass must retain interleaved PC/soldier insertion order"
        );
    }
}

#[test]
fn mixed_enemy_fifo_survives_detectable_mutation_between_entries() {
    use crate::ai::{AiLockFlags, StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    crate::sim_rng::with_seed(0xA013_F1F1, |sim| {
        engine.tick_enemy_ai(sim, &assets);

        // Consume the first retained entry, then model the detectable-list
        // mutation that Original explicitly postpones until after its full
        // The detection FIFO has been built. The second queued VIEW must be
        // independent of the now-live list.
        let first = engine
            .get_entity_mut(observer_id)
            .and_then(Entity::ai_controller_mut)
            .expect("mixed-fifo observer retains its controller")
            .stimulus_queue
            .remove(0);
        assert_eq!(first.stimulus_type, StimulusType::EventView);
        assert_eq!(
            first.info,
            StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
        );

        let observer = engine
            .get_entity_mut(observer_id)
            .and_then(Entity::npc_data_mut)
            .expect("mixed-fifo observer retains NPC state");
        observer.detectable_lists[DetectableType::Enemy as usize]
            .retain(|detectable| detectable.element != Some(royalist_id));
        observer
            .ai_brain
            .base_mut()
            .expect("mixed-fifo observer retains AI state")
            .locks_flag_field = AiLockFlags::empty();

        engine.tick_ai_queued_stimuli(sim, &assets);
    });

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("mixed-fifo observer retains EnemyAi after replay");
    assert_eq!(
        ai.base.last_stimulus_actor,
        Some(crate::ai::AiEntityHandle::new(royalist_id.index())),
        "later mixed VIEW must already be queued before an earlier Think can mutate detectables"
    );
}

#[test]
#[should_panic(expected = "eligible autonomous PC 0 has no EnemyAi brain during detection")]
fn autonomous_pc_detection_rejects_friendly_ai_brain() {
    use crate::element::{AiActorData, AiBrain, Entity};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("wrong-AI autonomous PC exists")
    else {
        panic!("wrong-AI autonomous PC changed kind")
    };
    pc.element.active = true;
    pc.pc.life_points = 100;
    pc.pc.ai = Some(Box::new(AiActorData {
        ai_brain: AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(pc_id.index()))),
        ..AiActorData::default()
    }));

    let _ = engine.enemy_optical_viewer_context_for_test(pc_id);
}

#[test]
fn lacklandist_mixed_enemy_cadence_is_selected_per_entry() {
    use crate::ai::{AlertLevel, StimulusInfo, StimulusType};
    use crate::element::Entity;

    fn observed_targets(frame: u32) -> Vec<crate::ai::AiEntityHandle> {
        let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
        engine.control.frame_counter = frame;
        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("cadence observer exists")
        else {
            panic!("cadence observer changed kind")
        };
        observer.npc.eye_status = crate::element::EyeStatus::LookForward;
        observer
            .npc
            .ai_brain
            .base_mut()
            .expect("cadence observer retains AI state")
            .current_music_alert_status = AlertLevel::Green;

        crate::sim_rng::with_seed(0xA013_CADE, |sim| engine.tick_enemy_ai(sim, &assets));

        let ai = engine
            .get_entity(observer_id)
            .and_then(Entity::ai_controller)
            .expect("cadence observer retains controller");
        let targets = ai
            .stimulus_queue
            .iter()
            .filter(|&stimulus| stimulus.stimulus_type == StimulusType::EventView)
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("cadence VIEW lost its human target")
                };
                target
            })
            .collect::<Vec<_>>();
        let expected = if frame == 3 {
            vec![crate::ai::AiEntityHandle::new(pc_id.index())]
        } else {
            vec![
                crate::ai::AiEntityHandle::new(pc_id.index()),
                crate::ai::AiEntityHandle::new(royalist_id.index()),
            ]
        };
        assert_eq!(targets, expected);
        targets
    }

    // Observer slot 0 has Original creation order 31. Frame 3 opens only
    // the modulo-2 PC cadence; frame 1 opens both modulo-2 and modulo-16.
    assert_eq!(observed_targets(3).len(), 1);
    assert_eq!(observed_targets(1).len(), 2);
}

#[test]
fn persisted_lean_out_flag_controls_detection_sharpness_after_posture_changes() {
    use crate::ai::AlertLevel;
    use crate::element::{DetectableType, Entity, EyeStatus, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer slot 0 has Original creation order 31, so modified frame 33
    // closes the modulo-2 PC cadence and reuses the exact cached visibility.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("lean-out observer exists")
    else {
        panic!("lean-out observer changed kind")
    };
    observer.element.publish_order_posture(Posture::Upright);
    observer.npc.eye_status = EyeStatus::LookForward;
    // The original game clears the lean-out state only while replacing
    // EYES_LOOK_DOWNWARDS. If another path already selected LookForward, the
    // serialized flag remains true even though posture is now Upright.
    observer.npc.view_lean_out = true;
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("lean-out observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    for (kind, list) in observer.npc.detectable_lists.iter_mut().enumerate() {
        if kind == DetectableType::Enemy as usize {
            list.retain(|detectable| detectable.element == Some(pc_id));
        } else {
            list.clear();
        }
    }
    let detectable = observer.npc.detectable_lists[DetectableType::Enemy as usize]
        .first_mut()
        .expect("lean-out observer tracks PC");
    detectable.last_visibility = 1.0;
    detectable.seen_now = true;
    detectable.seen_last_frame = true;

    crate::sim_rng::with_seed(0xA013_1A11, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("lean-out observer remains present")
    else {
        panic!("lean-out observer changed kind")
    };
    assert_eq!(observer.element.posture(), Posture::Upright);
    assert!(observer.npc.view_lean_out);
    assert_eq!(
        observer
            .npc
            .ai_brain
            .base()
            .expect("lean-out observer retains AI state")
            .max_visibility,
        u32::from(crate::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED),
        "Original selects the sharpness multiplier from bLeanOut, not posture"
    );
}

#[test]
fn persisted_lean_out_flag_controls_non_enemy_detection_sharpness() {
    use crate::ai::AlertLevel;
    use crate::element::{Detectable, DetectableType, Entity, EyeStatus, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    // Observer creation order 31 plus universal frame 2 produces modified
    // frame 33. Body's modulo-8 cadence is therefore closed, making the
    // persisted visibility sample the exact input to sharpness conversion.
    engine.control.frame_counter = 2;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("non-Enemy lean-out observer exists")
    else {
        panic!("non-Enemy lean-out observer changed kind")
    };
    observer.element.publish_order_posture(Posture::Upright);
    observer.npc.eye_status = EyeStatus::LookForward;
    observer.npc.view_lean_out = true;
    observer
        .npc
        .ai_brain
        .base_mut()
        .expect("non-Enemy lean-out observer retains AI state")
        .view_alert_status = AlertLevel::Green;
    for list in &mut observer.npc.detectable_lists {
        list.clear();
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Body,
        last_visibility: 1.0,
        seen_now: true,
        seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_1A12, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("non-Enemy lean-out observer remains present")
    else {
        panic!("non-Enemy lean-out observer changed kind")
    };
    assert_eq!(observer.element.posture(), Posture::Upright);
    assert!(observer.npc.view_lean_out);
    assert_eq!(
        observer
            .npc
            .ai_brain
            .base()
            .expect("non-Enemy lean-out observer retains AI state")
            .max_visibility,
        u32::from(crate::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED),
        "non-Enemy buckets use persisted bLeanOut for sharpness too"
    );
}

#[test]
fn enemy_optics_reads_pc_order_from_live_creation_slot_state() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};
    use crate::order::OrderType;

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    let mut element = crate::sequence::SequenceElement::new_generic(
        1,
        crate::element::Command::Wait,
        Some(pc_id),
    );
    element.state = crate::sequence::SequenceState::InProgress;
    element.push_order(crate::order::Order::test_new(
        OrderType::SimulatingBeggar,
        0.0,
        0.0,
    ));
    let seq_id = engine.orders.sequence_manager.launch_element(element);
    let elem_idx = 0;

    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("live-order observer retains NPC state");
    observer.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer
        .ai_brain
        .base_mut()
        .expect("live-order observer retains AI state")
        .got_the_beggar_trick = false;

    crate::sim_rng::with_seed(0xA013_11E0, |sim| {
        engine.refresh_detection_after_world_snapshot_for_test(sim, &assets, |engine| {
            engine
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                .expect("live-order sequence survives snapshot")
                .orders
                .front_mut()
                .expect("live-order sequence retains a front order")
                .order_type = OrderType::TransitionWaitingUprightSimulatingBeggar;
        });
    });

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::ai_controller)
        .expect("live-order observer retains AI controller");
    assert!(
        ai.stimulus_queue.iter().any(|stimulus| {
            stimulus.stimulus_type == StimulusType::EventView
                && stimulus.info
                    == StimulusInfo::Human(crate::ai::AiEntityHandle::new(pc_id.index()))
        }),
        "the live beggar transition must replace the snapshotted resting disguise"
    );
    assert!(
        ai.got_the_beggar_trick,
        "seeing the live beggar transition must teach the observer the disguise trick"
    );
}

#[test]
fn enemy_optics_reads_pc_detection_z_from_live_creation_slot_posture() {
    use crate::element::{DetectableType, Entity, Posture};

    let (mut engine, assets, observer_id, pc_id, _) = mixed_enemy_fifo_fixture(true);
    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("live-Z PC exists") else {
        panic!("live-Z target changed kind")
    };
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(15.0, 0.0, 20.0));
    pc.element
        .set_position_map_preserving_3d(MapPoint::new(15.0, -20.0));
    pc.element.publish_order_posture(Posture::Upright);

    let observer = engine
        .get_entity_mut(observer_id)
        .and_then(Entity::npc_data_mut)
        .expect("live-Z observer retains NPC state");
    observer.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer
        .ai_brain
        .base_mut()
        .expect("live-Z observer retains AI state")
        .got_the_beggar_trick = true;

    crate::sim_rng::with_seed(0xA013_11E1, |sim| {
        engine.refresh_detection_after_world_snapshot_for_test(sim, &assets, |engine| {
            let Entity::Pc(pc) = engine
                .get_entity_mut(pc_id)
                .expect("live-Z PC survives snapshot")
            else {
                panic!("live-Z target changed kind after snapshot")
            };
            pc.element.publish_order_posture(Posture::Crouched);
        });
    });

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("live-Z observer retains NPC state");
    let detectable = observer.detectable_lists[DetectableType::Enemy as usize]
        .iter()
        .find(|detectable| detectable.element == Some(pc_id))
        .expect("live-Z observer retains the PC detectable");
    assert_eq!(
        detectable.last_visibility, 2.0,
        "live crouched detection Z must satisfy the 3D close-visibility gate"
    );
}

#[test]
fn lacklandist_enemy_optics_keeps_but_cannot_see_hollow_man() {
    use crate::ai::{StimulusInfo, StimulusType};
    use crate::element::{DetectableType, Entity};

    let (mut engine, assets, observer_id, pc_id, royalist_id) = mixed_enemy_fifo_fixture(true);
    engine
        .get_entity_mut(pc_id)
        .and_then(Entity::human_data_mut)
        .expect("hollow target retains human state")
        .hollow_man = true;

    crate::sim_rng::with_seed(0xA013_4011, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("hollow observer retains NPC state");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize].len(),
        2,
        "HollowMan is invisible, not cleaned up"
    );
    assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_now);
    let ai = observer
        .ai_brain
        .base()
        .expect("hollow observer retains AI state");
    assert_eq!(ai.stimulus_queue.len(), 1);
    assert_eq!(ai.stimulus_queue[0].stimulus_type, StimulusType::EventView);
    assert_eq!(
        ai.stimulus_queue[0].info,
        StimulusInfo::Human(crate::ai::AiEntityHandle::new(royalist_id.index()))
    );
}
