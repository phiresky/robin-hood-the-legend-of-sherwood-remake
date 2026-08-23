use super::*;

/// Build a minimal soldier entity for posture / command tests.
pub(super) fn make_test_soldier(posture: crate::element::Posture) -> Entity {
    Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    })
}

#[test]
fn add_entity_assigns_original_script_element_index() {
    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    for id in [first, second] {
        assert_eq!(
            u32::from(
                engine
                    .get_entity(id)
                    .expect("inserted entity exists")
                    .element_data()
                    .index_in_elements_list
            ),
            id.index()
        );
    }
}

#[test]
fn owner_boundary_positions_follow_original_creation_order_not_entity_slots() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::entities::{BoundaryPosition, EntitySlots};
    use std::collections::BTreeMap;

    let mut engine = EngineInner::new();
    // Deliberately allocate in the opposite order from Original's element
    // walk. Rust slots are loader/runtime storage identities; Original
    // Hourglass visibility is determined by creation order.
    let later_target = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let owner = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let earlier_target = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
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
        engine.boundary_position(earlier_target, owner, &before, true),
        BoundaryPosition::of(engine.get_entity(earlier_target).unwrap().element_data()),
        "an earlier Original slot has already completed its actor movement"
    );
    assert_eq!(
        engine.boundary_position(later_target, owner, &before, true),
        later_before,
        "a later Original slot still exposes its preserved pre-movement position"
    );
    assert_eq!(
        engine.boundary_position(owner, owner, &before, false),
        owner_before,
        "the owner itself is pre-movement before its Actor Hourglass arm"
    );
    assert_eq!(
        engine.boundary_position(owner, owner, &before, true),
        BoundaryPosition::of(engine.get_entity(owner).unwrap().element_data()),
        "the owner itself is live after its Actor Hourglass arm"
    );
}

const SPEECH_TIMING_PROFILE_ID: u32 = 0x1234_0000;

fn build_mytalk_timing_test() -> (EngineInner, EntityId, LevelAssets) {
    use crate::ai::{Remark, SpeechFlags};
    use crate::element::AiBrain;
    use crate::profiles::SoldierProfile;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;

    let mut soldier_entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("timing-test soldier has EnemyAi")
        .hth_weapon_id = 1;
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.say_with_flags(Remark::Arrow, SpeechFlags::MYTALK_1 | SpeechFlags::ALWAYS);
    let soldier_id = engine.add_entity(soldier_entity);

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(SoldierProfile {
            profile_name: "timing-test-soldier".into(),
            exclamation_id: SPEECH_TIMING_PROFILE_ID,
            hth_weapon_id: 1,
            ..Default::default()
        });
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .hth_weapons
        .push(Default::default());
    (engine, soldier_id, assets)
}

fn mytalk_ai(engine: &EngineInner, soldier_id: EntityId) -> &crate::ai::AiController {
    engine
        .get_entity(soldier_id)
        .and_then(Entity::ai_controller)
        .expect("timing-test soldier has an AI controller")
}

#[test]
fn mytalk_completion_obeys_exact_asset_duration_frame() {
    use crate::ai::{LogLineType, Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
    assert_eq!(engine.feedback.sound_sim.pending_exclamations.len(), 1);
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 3,
    }]);
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame,
        103
    );
    assert_eq!(mytalk_ai(&engine, soldier_id).current_remark, Remark::Arrow);

    for frame in [101, 102] {
        engine.control.frame_counter = frame;
        super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, frame);
        engine.settle_npc_speech_completions(&sim, &assets);
        let ai = mytalk_ai(&engine, soldier_id);
        assert_eq!(ai.current_remark, Remark::Arrow);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    engine.control.frame_counter = 103;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
    engine.settle_npc_speech_completions(&sim, &assets);
    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert_eq!(
        ai.ai_log.last().map(|line| (line.line_type, line.info)),
        Some((LogLineType::Event, StimulusType::EventMyTalk1 as u16))
    );
}

#[test]
fn mytalk_uses_concrete_sound_manager_resolution_duration() {
    use crate::ai::Remark;

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 7,
    }]);
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame,
        107
    );
    assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
}

#[test]
fn replay_host_resolution_without_logical_request_keeps_authoritative_completion_timing() {
    use crate::ai::Remark;

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
    assert_eq!(
        mytalk_ai(&engine, soldier_id).current_remark,
        Remark::TheSoundOfSilence
    );

    engine.queue_replay_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 3,
    }]);
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame,
        103
    );

    engine.control.frame_counter = 103;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
    engine.settle_npc_speech_completions(&sim, &assets);
    assert!(engine.feedback.sound_sim.playing_exclamations.is_empty());
    assert_eq!(
        mytalk_ai(&engine, soldier_id).current_remark,
        Remark::TheSoundOfSilence,
        "an Original-only host line must not invent a Rust AI speech latch"
    );
}

#[test]
fn replay_host_resolution_preserves_an_unrelated_reconstructed_pending_request() {
    use crate::ai::Remark;
    use crate::sound::{ExclamationGroup, PendingExclamation};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    let unrelated = PendingExclamation {
        actor_id: 45,
        group: ExclamationGroup::Civilian,
        profile_id: 0x5755_0000,
        exclamation_id: 16,
        variant: -1,
    };
    engine
        .feedback
        .sound_sim
        .pending_exclamations
        .push(unrelated.clone());

    engine.queue_replay_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 3,
    }]);
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    assert_eq!(engine.feedback.sound_sim.pending_exclamations.len(), 1);
    let retained = &engine.feedback.sound_sim.pending_exclamations[0];
    assert_eq!(
        (
            retained.actor_id,
            retained.profile_id,
            retained.exclamation_id,
            retained.variant,
        ),
        (
            unrelated.actor_id,
            unrelated.profile_id,
            unrelated.exclamation_id,
            unrelated.variant,
        ),
        "an Original-only host completion must not consume unrelated reconstructed Rust speech"
    );
    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].actor_id,
        soldier_id.index()
    );
    assert_eq!(
        mytalk_ai(&engine, soldier_id).current_remark,
        Remark::TheSoundOfSilence,
        "an unmatched host completion must not invent a logical AI remark"
    );
}

#[test]
#[should_panic(expected = "live sound manager resolved exclamation")]
fn live_host_resolution_without_logical_request_remains_an_invariant_failure() {
    use crate::ai::Remark;

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 3,
    }]);
    engine.hourglass_phase_deferred_effects_start(&crate::sim_rng::test_context(), &assets);
}

#[test]
fn zero_duration_resolution_completes_mytalk_at_current_boundary() {
    use crate::ai::{LogLineType, Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 0,
    }]);
    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(ai.current_remark, Remark::Arrow);

    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(engine.control.frame_counter, 100);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert_eq!(
        ai.ai_log.last().map(|line| (line.line_type, line.info)),
        Some((LogLineType::Event, StimulusType::EventMyTalk1 as u16))
    );
}

#[test]
fn actor_effect_prefix_does_not_consume_caller_tail_self_stimulus() {
    use crate::ai::{
        AiActorOutbox, AiOwnerWork, AiState, AiStateChangeNotification, AiStateChangeSource,
        StimulusType, Substate,
    };
    use crate::element::AiBrain;

    let mut engine = EngineInner::new();
    let mut soldier_entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let ai = soldier.npc.ai_brain.base_mut().expect("test soldier AI");
    let mut prefix = AiActorOutbox::default();
    prefix.unfocus = true;
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(prefix));
    let mut halt_before_callback = AiActorOutbox::default();
    halt_before_callback.queue_halt();
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::StateChange(AiStateChangeNotification {
            outgoing_state: AiState::Default,
            outgoing_substate: Substate::DefaultOnPost,
            incoming_state: AiState::Seeking,
            incoming_substate: Substate::SeekingBody,
            source: AiStateChangeSource::SelfActor,
            actor_effects_before_callback: Some(halt_before_callback),
        }));
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventTimer.into());
    let soldier_id = engine.add_entity(soldier_entity);

    engine.drain_ai_owner_work_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        soldier_id,
    );

    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::ai_controller)
        .expect("test soldier AI survives prefix drain");
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventTimer],
        "Focus(NULL)'s recursive ActorEffects boundary must leave the caller-tail event behind the older Halt"
    );
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(!ai.outbox.actor.halt);
}

#[test]
fn set_state_halt_prefix_retains_detached_goto_until_engine_rejection() {
    use crate::ai::{
        AiActorOutbox, AiOwnerWork, AiState, AiStateChangeNotification, AiStateChangeSource,
        StimulusType, Substate,
    };
    use crate::element::{AiBrain, Posture};
    use crate::order::{AiOrderIntent, OrderType};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = crate::coordinates::MapSize::new(100.0, 100.0);

    let mut soldier_entity = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position_map(crate::coordinates::MapPoint::new(90.0, 90.0));
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(soldier_entity);

    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has Enemy AI");
        ai.think_recursion_depth = 1;
        ai.completion_latch_inside_think = true;

        // SeekingSeekpoint calls Halt, SetState, then GoTo. SetState stores
        // the Halt in its pre-callback prefix while the later GoTo remains in
        // the caller tail until that prefix has settled.
        let mut halt_prefix = AiActorOutbox::default();
        halt_prefix.queue_halt();
        ai.outbox
            .reentrant
            .owner_work
            .push(AiOwnerWork::StateChange(AiStateChangeNotification {
                outgoing_state: AiState::Seeking,
                outgoing_substate: Substate::SeekingGroupGetInstructedByOfficer,
                incoming_state: AiState::Seeking,
                incoming_substate: Substate::SeekingSeekpoint,
                source: AiStateChangeSource::SelfActor,
                actor_effects_before_callback: Some(halt_prefix),
            }));
        ai.outbox
            .actor
            .orders
            .push(AiOrderIntent::new(OrderType::RunningUpright, 100.0, 90.0));
        assert!(ai.end_think_completion_events());
    }

    engine.drain_ai_owner_work_for(&sim, &assets, owner);
    {
        let ai = engine
            .get_entity(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI after SetState prefix");
        assert_eq!(ai.think_recursion_depth, 1);
        assert!(ai.completion_latch_inside_think);
        assert_eq!(ai.outbox.actor.orders.len(), 1);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    // A prefix drain may reach the generic completion surface before the
    // caller-tail GoTo has been handed to movement. It must not mistake the
    // absence of a result for successful authorization and close EndThink.
    engine.surface_synchronous_completion_events_for_owner(owner);
    {
        let ai = engine
            .get_entity(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI while its GoTo verdict is pending");
        assert_eq!(ai.think_recursion_depth, 1);
        assert!(ai.completion_latch_inside_think);
        assert_eq!(ai.engine_deferred_end_think_frames, 1);
        assert_eq!(ai.outbox.actor.orders.len(), 1);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    // The caller-tail GoTo is outside the level and is rejected only after
    // the AI borrow is released. Original AppendMoveToSequence reports this
    // inline to the enclosing EndThink, which recursively dispatches the
    // fallback seek event.
    engine.launch_pending_orders_for_npc_mode(&sim, &assets, owner, false);
    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("test soldier retains AI after rejected GoTo");
    assert_eq!(ai.think_recursion_depth, 1);
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventCouldntReachPoint]
    );
}

#[test]
fn pre_set_state_face_and_attentive_leave_register_then_preempt_in_manager_fifo() {
    use crate::ai::{
        AiActorOutbox, AiOwnerWork, AiState, AiStateChangeNotification, AiStateChangeSource,
        AttentiveModeEffect, Substate,
    };
    use crate::element::{AiBrain, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier_entity = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let enemy = soldier.npc.ai_brain.enemy_mut().expect("Enemy test AI");
    enemy.attentive = true;
    enemy.will_be_attentive = true;
    enemy.base.current_state = AiState::Default;
    enemy.base.current_substate = Substate::DefaultGotoPostTurn;
    let owner = engine.add_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut face_prefix = AiActorOutbox::default();
    face_prefix
        .orders
        .push(crate::order::AiOrderIntent::face_direction(7));
    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("Enemy test AI remains live");
        ai.outbox
            .reentrant
            .owner_work
            .push(AiOwnerWork::StateChange(AiStateChangeNotification {
                outgoing_state: AiState::Default,
                outgoing_substate: Substate::DefaultGotoPost,
                incoming_state: AiState::Default,
                incoming_substate: Substate::DefaultGotoPostTurn,
                source: AiStateChangeSource::SelfActor,
                actor_effects_before_callback: Some(face_prefix),
            }));
        ai.outbox
            .actor
            .queue_set_attentive_mode(AttentiveModeEffect::new(false, false));
    }

    // This is the movement-condolation mode that exposed the bug. Face and
    // SetAttentiveMode both launch inline, but Original leaves their ordinary
    // elements registered until the global sequence-manager Hourglass.
    engine.drain_direct_ai_owner_boundary_mode(&sim, owner, &assets, true, true);

    let owned_before_manager: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| (element.command, element.state))
        .collect();
    assert_eq!(
        owned_before_manager,
        [
            (Command::Turn, SequenceState::Todo),
            (Command::LeaveAttentiveMode, SequenceState::Todo),
        ],
        "Face must register before SetState's attentive tail without instructing either in the owner slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .len(),
        2
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .is_none(),
        "registered Face/Leave elements must not become actor-selected during the owner slot"
    );
    let enemy = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("Enemy test AI remains live");
    assert!(
        !enemy.will_be_attentive,
        "SetAttentiveMode updates its gate immediately"
    );

    // Repeating the already-requested target must observe will_be_attentive
    // and must not append a duplicate deferred Leave.
    engine.set_soldier_attentive_mode(owner, false, false);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .len(),
        2
    );

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let owned_after_manager: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| (element.command, element.state))
        .collect();
    assert!(
        owned_after_manager.contains(&(Command::Turn, SequenceState::Postponed)),
        "manager FIFO must start Face first, then let Leave postpone it; owned={owned_after_manager:?}"
    );
    assert!(
        owned_after_manager.contains(&(Command::LeaveAttentiveMode, SequenceState::InProgress)),
        "manager FIFO must leave the later attentive transition authoritative"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sequence, index)| engine
                .orders
                .sequence_manager
                .get_element(sequence, index))
            .map(|element| element.command),
        Some(Command::LeaveAttentiveMode)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::TransitionWaitingAlertedWaitingUpright)
    );
}

#[test]
fn consecutive_set_states_preserve_attentive_request_fifo() {
    use crate::ai::{AiState, Substate};
    use crate::element::{AiBrain, Command, Posture};
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier_entity = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let enemy = soldier.npc.ai_brain.enemy_mut().expect("Enemy test AI");
    enemy.attentive = true;
    enemy.will_be_attentive = true;
    enemy.base.current_state = AiState::Seeking;
    enemy.base.current_substate = Substate::SeekingSeekpoint;
    enemy.base.stop_all();
    enemy.set_state(AiState::Attacking, Substate::AttackingReactiontime);
    enemy.set_state(
        AiState::Attacking,
        Substate::AttackingTooProudToAttackApproach,
    );
    let owner = engine.add_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_direct_ai_owner_boundary_mode(&sim, owner, &assets, true, true);

    let owned: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| (element.command, element.state))
        .collect();
    assert_eq!(
        owned,
        [(Command::LeaveAttentiveMode, SequenceState::Todo)],
        "Reactiontime's attentive=true observes the already-true will-be gate, then the immediately following TooProudApproach attentive=false launches the sole transition"
    );
    let enemy = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("Enemy test AI remains live");
    assert!(enemy.attentive);
    assert!(!enemy.will_be_attentive);
    assert_eq!(enemy.base.current_state, AiState::Attacking);
    assert_eq!(
        enemy.base.current_substate,
        Substate::AttackingTooProudToAttackApproach
    );
}

#[test]
fn opposite_attentive_transitions_launch_before_following_turn() {
    use crate::ai::AttentiveModeEffect;
    use crate::element::{AiBrain, Command, Posture};
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier_entity = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let enemy = soldier.npc.ai_brain.enemy_mut().expect("Enemy test AI");
    enemy.attentive = false;
    enemy.will_be_attentive = false;
    enemy
        .base
        .outbox
        .actor
        .queue_set_attentive_mode(AttentiveModeEffect::new(true, false));
    enemy
        .base
        .outbox
        .actor
        .queue_set_attentive_mode(AttentiveModeEffect::new(false, false));
    let mut turn = crate::order::AiOrderIntent::face_direction(14);
    turn.after_attentive_mode = true;
    enemy.base.outbox.actor.orders.push(turn);
    let owner = engine.add_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_direct_ai_owner_boundary_mode(&sim, owner, &assets, true, true);

    let owned: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| (element.command, element.state))
        .collect();
    assert_eq!(
        owned,
        [
            (Command::EnterAttentiveMode, SequenceState::Todo),
            (Command::LeaveAttentiveMode, SequenceState::Todo),
            (Command::Turn, SequenceState::Todo),
        ],
        "Original launches both opposite SetAttentiveMode transitions synchronously before the following Face"
    );
    assert!(
        !engine
            .get_entity(owner)
            .and_then(Entity::enemy_ai)
            .expect("Enemy test AI remains live")
            .will_be_attentive,
        "the final attentive request still owns the projected flag"
    );
}

#[test]
fn matured_mytalk_completion_precedes_deferred_hades_replacement() {
    use crate::ai::{LogLineType, Remark};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: soldier_id.index(),
        identifier: (SPEECH_TIMING_PROFILE_ID & 0xFFFF_0000) | u32::from(Remark::Arrow as u16),
        exclamation_id: Remark::Arrow as u16,
        duration_frames: 3,
    }]);
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);
    engine.control.frame_counter = 103;
    engine.orders.pending_hades_kills.push(soldier_id);

    engine.hourglass_phase_deferred_effects_start(&sim, &assets);

    let log = speech_log(&engine, soldier_id);
    let finished = log
        .iter()
        .position(|(kind, _)| *kind == LogLineType::SpeakFinished)
        .expect("matured line completes before deferred HADES mutates the actor");
    if let Some(death_speech) = log
        .iter()
        .position(|entry| *entry == (LogLineType::Speak, Remark::Dies as u16))
    {
        assert!(finished < death_speech);
    }
}

#[test]
fn stop_exclamation_cancels_unresolved_request_before_fifo_resolution() {
    use crate::ai::Remark;

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test();
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
    assert_eq!(engine.feedback.sound_sim.pending_exclamations.len(), 1);

    engine.cancel_exclamation_callbacks(soldier_id.index());
    assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());

    engine.queue_resolved_exclamations(Vec::new());
    engine.hourglass_phase_deferred_effects_start(&sim, &assets);
    assert!(engine.feedback.sound_sim.playing_exclamations.is_empty());
    assert_eq!(mytalk_ai(&engine, soldier_id).current_remark, Remark::Arrow);
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

#[derive(Clone, Copy)]
enum SpeechNpcKind {
    Soldier { vip: bool },
    Civilian { vip: bool },
}

fn add_speech_test_npc(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    kind: SpeechNpcKind,
    speech_id: u32,
) -> EntityId {
    match kind {
        SpeechNpcKind::Soldier { vip } => {
            let profile_index = {
                let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
                if profiles.hth_weapons.is_empty() {
                    profiles.hth_weapons.push(Default::default());
                }
                let index = profiles.soldiers.len() as u32;
                profiles.soldiers.push(crate::profiles::SoldierProfile {
                    profile_name: format!("soldier-{index}"),
                    exclamation_id: speech_id,
                    hth_weapon_id: 1,
                    vip,
                    ..Default::default()
                });
                crate::profiles::SoldierProfileIdx(index)
            };
            let mut entity = make_test_soldier(crate::element::Posture::Upright);
            let Entity::Soldier(soldier) = &mut entity else {
                unreachable!()
            };
            soldier.soldier.soldier_profile_index = profile_index;
            soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
            soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("speech soldier has EnemyAi")
                .hth_weapon_id = 1;
            engine.add_entity(entity)
        }
        SpeechNpcKind::Civilian { vip } => {
            let profile_index = {
                let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
                let index = profiles.civilians.len() as u32;
                profiles.civilians.push(crate::profiles::CivilianProfile {
                    profile_name: format!("civilian-{index}"),
                    exclamation_id: speech_id,
                    civilian_type: if vip {
                        crate::profiles::CivilianType::Vip
                    } else {
                        crate::profiles::CivilianType::Man
                    },
                    ..Default::default()
                });
                crate::profiles::CivilianProfileIdx(index)
            };
            let mut entity = make_test_civilian(crate::element::Posture::Upright);
            let Entity::Civilian(civilian) = &mut entity else {
                unreachable!()
            };
            civilian.civilian.civilian_profile_index = profile_index;
            civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::default());
            engine.add_entity(entity)
        }
    }
}

fn queue_and_settle_speech(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    owner: EntityId,
    remark: crate::ai::Remark,
    flags: crate::ai::SpeechFlags,
) {
    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("speech test owner has AI")
        .say_with_flags(remark, flags);
    engine.drain_ai_owner_work_for(&crate::sim_rng::test_context(), assets, owner);
}

fn speech_log(engine: &EngineInner, owner: EntityId) -> Vec<(crate::ai::LogLineType, u16)> {
    engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("speech test owner has AI")
        .ai_log
        .iter()
        .map(|line| (line.line_type, line.info))
        .collect()
}

fn last_speech_impossible(engine: &EngineInner, owner: EntityId) -> Option<u16> {
    speech_log(engine, owner)
        .into_iter()
        .rev()
        .find_map(|(kind, info)| (kind == crate::ai::LogLineType::SpeakImpossible).then_some(info))
}

fn exclamation_for(
    engine: &EngineInner,
    owner: EntityId,
) -> Option<(crate::sound::ExclamationGroup, u32, u16, i32)> {
    engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .rev()
        .find_map(|command| match command {
            crate::engine::SoundCommand::Exclamation {
                group,
                profile_id,
                exclamation_id,
                variant,
                actor_id: Some(actor_id),
                ..
            } if *actor_id == owner => Some((*group, *profile_id, *exclamation_id, *variant)),
            _ => None,
        })
}

#[test]
fn speech_family_matrix_matches_original_category_banks_and_reasons() {
    use crate::ai::{Remark, SpeechFlags};
    use crate::sound::ExclamationGroup;

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let ordinary_soldier = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        101,
    );
    let vip_soldier = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: true },
        102,
    );
    let ordinary_civilian = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Civilian { vip: false },
        201,
    );
    let vip_civilian = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Civilian { vip: true },
        202,
    );

    queue_and_settle_speech(
        &mut engine,
        &assets,
        ordinary_soldier,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        ordinary_civilian,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(last_speech_impossible(&engine, ordinary_soldier), Some(5));
    assert_eq!(last_speech_impossible(&engine, ordinary_civilian), Some(6));
    queue_and_settle_speech(
        &mut engine,
        &assets,
        vip_soldier,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        vip_civilian,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS,
    );
    let vip_warcry_id = (Remark::VipWarcry as u32 - Remark::FIRST_VIP as u32) as u16;
    assert_eq!(
        exclamation_for(&engine, vip_soldier),
        Some((ExclamationGroup::Vip, 102, vip_warcry_id, -1))
    );
    assert_eq!(
        exclamation_for(&engine, vip_civilian),
        Some((ExclamationGroup::Vip, 202, vip_warcry_id, -1))
    );

    for (owner, reason) in [(ordinary_soldier, 7), (vip_soldier, 7), (vip_civilian, 8)] {
        // Clear the accepted VIP lines so this arm reaches category dispatch.
        if let Some(ai) = engine
            .get_entity_mut(owner)
            .and_then(Entity::ai_controller_mut)
        {
            ai.current_remark = Remark::TheSoundOfSilence;
            ai.current_remark_flags = 0;
        }
        queue_and_settle_speech(
            &mut engine,
            &assets,
            owner,
            Remark::CivPanic,
            SpeechFlags::ALWAYS,
        );
        assert_eq!(last_speech_impossible(&engine, owner), Some(reason));
    }
    queue_and_settle_speech(
        &mut engine,
        &assets,
        ordinary_civilian,
        Remark::CivPanic,
        SpeechFlags::ALWAYS,
    );
    let civ_panic_id = (Remark::CivPanic as u32 - Remark::FIRST_CIVILIAN as u32) as u16;
    assert_eq!(
        exclamation_for(&engine, ordinary_civilian),
        Some((ExclamationGroup::Civilian, 201, civ_panic_id, -1))
    );

    for owner in [ordinary_civilian, vip_civilian] {
        if let Some(ai) = engine
            .get_entity_mut(owner)
            .and_then(Entity::ai_controller_mut)
        {
            ai.current_remark = Remark::TheSoundOfSilence;
            ai.current_remark_flags = 0;
        }
        queue_and_settle_speech(
            &mut engine,
            &assets,
            owner,
            Remark::Arrow,
            SpeechFlags::ALWAYS,
        );
        assert_eq!(last_speech_impossible(&engine, owner), Some(9));
    }
    if let Some(ai) = engine
        .get_entity_mut(vip_soldier)
        .and_then(Entity::ai_controller_mut)
    {
        ai.current_remark = Remark::TheSoundOfSilence;
        ai.current_remark_flags = 0;
    }
    queue_and_settle_speech(
        &mut engine,
        &assets,
        vip_soldier,
        Remark::Arrow,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(last_speech_impossible(&engine, vip_soldier), Some(10));
    queue_and_settle_speech(
        &mut engine,
        &assets,
        ordinary_soldier,
        Remark::Arrow,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(
        exclamation_for(&engine, ordinary_soldier),
        Some((ExclamationGroup::Civilian, 101, Remark::Arrow as u16, -1))
    );
}

#[test]
fn category_rejection_preserves_callback_latch_and_reason_specific_log_order() {
    use crate::ai::{LogLineType, Remark, SpeechFlags, StimulusType};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let reason_five = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        211,
    );
    let reason_nine = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Civilian { vip: false },
        212,
    );

    queue_and_settle_speech(
        &mut engine,
        &assets,
        reason_five,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        reason_nine,
        Remark::Arrow,
        SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1,
    );

    let relevant = |owner| {
        speech_log(&engine, owner)
            .into_iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    LogLineType::Speak | LogLineType::SpeakImpossible | LogLineType::Event
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        relevant(reason_five),
        vec![
            (LogLineType::Speak, Remark::VipWarcry as u16),
            (LogLineType::SpeakImpossible, 5),
            (LogLineType::Event, StimulusType::EventMyTalk1 as u16),
        ]
    );
    assert_eq!(
        relevant(reason_nine),
        vec![
            (LogLineType::Speak, Remark::Arrow as u16),
            (LogLineType::Event, StimulusType::EventMyTalk1 as u16),
            (LogLineType::SpeakImpossible, 9),
        ]
    );
    for owner in [reason_five, reason_nine] {
        let ai = mytalk_ai(&engine, owner);
        assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
        assert_eq!(ai.current_remark_flags, 0);
    }
}

#[test]
fn category_rejection_tail_clears_recursive_emergency_line() {
    use crate::ai::{AiSpeechAttempt, Remark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        221,
    );
    let rejected_flags = SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1;
    let settlement = engine.settle_npc_speech_attempt(
        &assets,
        owner,
        AiSpeechAttempt {
            remark: Remark::VipWarcry,
            flags: rejected_flags.bits(),
        },
    );
    assert!(settlement.invoke_finished_callback);
    assert_eq!(mytalk_ai(&engine, owner).current_remark, Remark::VipWarcry);
    assert_eq!(
        mytalk_ai(&engine, owner).current_remark_flags,
        rejected_flags.bits(),
        "InformAIOnFinishedRemark observes the rejected Say latch"
    );

    let recursive = engine.settle_npc_speech_attempt(
        &assets,
        owner,
        AiSpeechAttempt {
            remark: Remark::Arrow,
            flags: (SpeechFlags::ALWAYS | SpeechFlags::EMERGENCY).bits(),
        },
    );
    assert_eq!(recursive, super::super::ai::NpcSpeechSettlement::default());
    assert_eq!(mytalk_ai(&engine, owner).current_remark, Remark::Arrow);

    engine.finalize_category_speech_rejection(
        owner,
        settlement
            .category_rejection
            .expect("category rejection has an unconditional return tail"),
    );
    let ai = mytalk_ai(&engine, owner);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert_eq!(ai.current_remark_flags, 0);
    assert!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .any(|line| line.actor_id == owner.index()
                && line.exclamation_id == Remark::Arrow as u16),
        "the recursive line started, but the outer rejected Say overwrote its latch"
    );
}

#[test]
fn speech_early_filter_order_and_always_bypass_are_exact() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 20;
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        301,
    );
    install_test_building_sector(&mut engine, 42);
    let building = crate::position_interface::SectorHandle::new(42).unwrap();
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.element_data_mut().blipped = true;
        entity.element_data_mut().set_sector(Some(building));
        let ai = entity.ai_controller_mut().unwrap();
        ai.forbidden_remark_ids.push(Remark::Arrow as u32);
    }
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::VILLAINS.bits(),
        speech_id: 0,
        guy_index: 0,
        bad_guy: true,
        forbidden_till_frame: 20,
    });

    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(0));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .blipped = false;
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(1));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .forbidden_remark_ids
        .clear();
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(2));
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(3));
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::ALWAYS | SpeechFlags::HOUSE,
    );
    assert_eq!(mytalk_ai(&engine, owner).current_remark, Remark::Arrow);
}

#[test]
fn shared_cycle_advances_after_early_filters_but_before_busy_category_and_id_zero() {
    use crate::ai::{Remark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let filtered = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        401,
    );
    engine
        .get_entity_mut(filtered)
        .unwrap()
        .element_data_mut()
        .blipped = true;
    queue_and_settle_speech(
        &mut engine,
        &assets,
        filtered,
        Remark::Arrow,
        SpeechFlags::CYCLE_3_VARIANTS,
    );
    assert_eq!(engine.ai.global.current_speech_variant, 0);

    let busy = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        402,
    );
    engine
        .get_entity_mut(busy)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .current_remark = Remark::Wounded;
    queue_and_settle_speech(
        &mut engine,
        &assets,
        busy,
        Remark::Arrow,
        SpeechFlags::CYCLE_3_VARIANTS | SpeechFlags::ALWAYS,
    );
    assert_eq!(engine.ai.global.current_speech_variant, 1);
    assert_eq!(last_speech_impossible(&engine, busy), Some(4));

    let mismatch = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Civilian { vip: true },
        403,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        mismatch,
        Remark::CivPanic,
        SpeechFlags::CYCLE_3_VARIANTS | SpeechFlags::ALWAYS,
    );
    assert_eq!(engine.ai.global.current_speech_variant, 2);
    assert_eq!(last_speech_impossible(&engine, mismatch), Some(8));

    let silent = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        0,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        silent,
        Remark::Arrow,
        SpeechFlags::CYCLE_3_VARIANTS | SpeechFlags::ALWAYS,
    );
    assert_eq!(engine.ai.global.current_speech_variant, 0);
    assert_eq!(mytalk_ai(&engine, silent).current_remark, Remark::Arrow);
}

#[test]
fn speech_fifo_preserves_rejected_accepted_busy_and_emergency_attempts() {
    use crate::ai::{ForbiddenRemark, LogLineType, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 50;
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        501,
    );
    let owner_creation_order = engine.world.original_creation_order(owner);
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: owner_creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: 50,
    });
    {
        let ai = engine
            .get_entity_mut(owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap();
        ai.say_with_flags(Remark::Arrow, SpeechFlags::empty());
        ai.say_with_flags(Remark::Arrow, SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1);
        ai.say_with_flags(
            Remark::WaspSting,
            SpeechFlags::ALWAYS | SpeechFlags::MYTALK_2,
        );
        ai.say_with_flags(
            Remark::Wounded,
            SpeechFlags::ALWAYS | SpeechFlags::EMERGENCY | SpeechFlags::MYTALK_3,
        );
    }
    engine.drain_ai_owner_work_for(&crate::sim_rng::test_context(), &assets, owner);

    assert_eq!(
        speech_log(&engine, owner),
        vec![
            (LogLineType::Speak, Remark::Arrow as u16),
            (LogLineType::SpeakImpossible, 2),
            (LogLineType::Speak, Remark::Arrow as u16),
            (LogLineType::Speak, Remark::WaspSting as u16),
            (LogLineType::SpeakImpossible, 4),
            (
                LogLineType::Event,
                crate::ai::StimulusType::EventMyTalk2 as u16,
            ),
            (LogLineType::Speak, Remark::Wounded as u16),
        ]
    );
    let ai = mytalk_ai(&engine, owner);
    assert_eq!(ai.current_remark, Remark::Wounded);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    // Accepted lines wait as pending requests until the concrete sound
    // manager resolves a duration; only then do they start playing.
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| pending.exclamation_id)
            .collect::<Vec<_>>(),
        vec![Remark::Wounded as u16]
    );
    engine.queue_resolved_exclamations(vec![crate::sound::ResolvedExclamation {
        actor_id: owner.index(),
        identifier: u32::from(Remark::Wounded as u16) | (501 & 0xFFFF_0000),
        exclamation_id: Remark::Wounded as u16,
        duration_frames: 5,
    }]);
    engine.hourglass_phase_deferred_effects_start(&crate::sim_rng::test_context(), &assets);
    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].exclamation_id,
        Remark::Wounded as u32
    );

    engine
        .feedback
        .sound_sim
        .finished_exclamations
        .push((owner.index(), Remark::Arrow as u32));
    engine.settle_npc_speech_completions(&crate::sim_rng::test_context(), &assets);
    assert_eq!(mytalk_ai(&engine, owner).current_remark, Remark::Wounded);
    assert!(
        !speech_log(&engine, owner)
            .iter()
            .any(|(kind, _)| *kind == LogLineType::SpeakFinished)
    );

    engine
        .feedback
        .sound_sim
        .finished_exclamations
        .push((owner.index(), Remark::Wounded as u32));
    engine.settle_npc_speech_completions(&crate::sim_rng::test_context(), &assets);
    assert_eq!(
        mytalk_ai(&engine, owner).current_remark,
        Remark::TheSoundOfSilence
    );
    assert!(
        speech_log(&engine, owner)
            .iter()
            .any(|(kind, _)| *kind == LogLineType::SpeakFinished)
    );
}

#[test]
fn send_charly_tail_runs_after_both_rejected_and_accepted_speech() {
    use crate::ai::{
        AiOwnerWork, AiState, ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags, Substate,
    };

    for rejected in [true, false] {
        let mut engine = EngineInner::new();
        engine.control.frame_counter = 50;
        let mut assets = LevelAssets::new();
        let owner = add_speech_test_npc(
            &mut engine,
            &mut assets,
            SpeechNpcKind::Soldier { vip: false },
            501,
        );
        let charly = add_speech_test_npc(
            &mut engine,
            &mut assets,
            SpeechNpcKind::Soldier { vip: false },
            502,
        );
        let charly_handle = charly.index();
        let owner_creation_order = engine.world.original_creation_order(owner);
        if rejected {
            engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
                remark: Remark::FoundCharly,
                flags: RemarkTargetFlags::THIS_GUY.bits(),
                speech_id: 0,
                guy_index: owner_creation_order as u16,
                bad_guy: true,
                forbidden_till_frame: 50,
            });
        }
        {
            let enemy = engine
                .get_entity_mut(owner)
                .and_then(Entity::enemy_ai_mut)
                .expect("speech test owner has Enemy AI");
            enemy.base.current_state = AiState::Seeking;
            enemy.base.current_substate = Substate::SeekingSendCharlyToOfficer;
            enemy.base.friend_in_trouble = 0;
            enemy
                .base
                .say_with_flags(Remark::FoundCharly, SpeechFlags::MYTALK_1);
            enemy
                .base
                .outbox
                .reentrant
                .owner_work
                .push(AiOwnerWork::ResumeSendCharlyAfterSpeech {
                    charly: charly_handle,
                });
        }

        engine.drain_ai_owner_work_for(&crate::sim_rng::test_context(), &assets, owner);

        let enemy = engine
            .get_entity(owner)
            .and_then(Entity::enemy_ai)
            .expect("speech test owner retains Enemy AI");
        assert_eq!(enemy.base.friend_in_trouble, charly_handle);
        if rejected {
            assert_eq!(enemy.base.current_state, AiState::Default);
            assert_eq!(last_speech_impossible(&engine, owner), Some(2));
        } else {
            assert_eq!(enemy.base.current_state, AiState::Seeking);
            assert_eq!(
                enemy.base.current_substate,
                Substate::SeekingSendCharlyToOfficer
            );
            assert_eq!(enemy.base.current_remark, Remark::FoundCharly);
        }
    }
}

#[test]
fn speech_id_zero_latches_subtitle_and_forbid_without_completion_callback() {
    use crate::ai::{Remark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        0,
    );
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite
        .frame_profile_name = "live-frame-profile".into();
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1,
    );
    let ai = mytalk_ai(&engine, owner);
    assert_eq!(ai.current_remark, Remark::Arrow);
    assert_eq!(
        ai.current_remark_flags,
        (SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1).bits()
    );
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert!(engine.feedback.sound_sim.playing_exclamations.is_empty());
    assert!(exclamation_for(&engine, owner).is_none());
    assert_eq!(engine.ai.global.screen_remarks.len(), 1);
    assert_eq!(engine.ai.global.screen_remarks[0].timer, 100);
    assert_eq!(engine.ai.global.screen_remarks[0].remark, Remark::Arrow);
    assert_eq!(
        engine.ai.global.screen_remarks[0].prefix,
        "live-frame-profile"
    );
    assert_ne!(engine.ai.global.screen_remarks[0].prefix, "soldier-0");
    assert_eq!(engine.ai.global.forbidden_remarks.len(), 1);
    assert_eq!(engine.ai.global.forbidden_remarks[0].speech_id, 0);
}

#[test]
#[should_panic(expected = "invalid automatic-forbid remark NumberOfRemarks")]
fn number_of_remarks_sentinel_fails_instead_of_entering_automatic_forbid() {
    use crate::ai::{Remark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        0,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::NumberOfRemarks,
        SpeechFlags::ALWAYS,
    );
}

#[test]
#[should_panic(expected = "invalid automatic-forbid remark TheSoundOfSilence")]
fn sound_of_silence_sentinel_fails_instead_of_entering_automatic_forbid() {
    use crate::ai::{Remark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        0,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::TheSoundOfSilence,
        SpeechFlags::ALWAYS,
    );
}

#[test]
fn forbidden_scan_is_lazy_ordered_and_equal_deadline_is_live() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 10;
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        601,
    );
    let owner_creation_order = engine.world.original_creation_order(owner);
    engine.ai.global.forbidden_remarks = vec![
        ForbiddenRemark {
            remark: Remark::WaspSting,
            flags: RemarkTargetFlags::VILLAINS.bits(),
            speech_id: 0,
            guy_index: 0,
            bad_guy: true,
            forbidden_till_frame: 9,
        },
        ForbiddenRemark {
            remark: Remark::Arrow,
            flags: RemarkTargetFlags::THIS_GUY.bits(),
            speech_id: 0,
            guy_index: owner_creation_order as u16,
            bad_guy: true,
            forbidden_till_frame: 10,
        },
        ForbiddenRemark {
            remark: Remark::Arrow,
            flags: RemarkTargetFlags::VILLAINS.bits(),
            speech_id: 0,
            guy_index: 0,
            bad_guy: true,
            forbidden_till_frame: 8,
        },
    ];
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(2));
    assert_eq!(engine.ai.global.forbidden_remarks.len(), 2);
    assert_eq!(
        engine.ai.global.forbidden_remarks[0].forbidden_till_frame,
        10
    );
    assert_eq!(
        engine.ai.global.forbidden_remarks[1].forbidden_till_frame,
        8
    );

    let before = engine.ai.global.forbidden_remarks.clone();
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(engine.ai.global.forbidden_remarks.len(), before.len() + 1);
    assert_eq!(
        serde_json::to_value(&engine.ai.global.forbidden_remarks[..before.len()]).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "ALWAYS skips the lazy scan, including expired-entry deletion"
    );
}

#[test]
fn this_guy_forbid_isolated_by_npc_creation_order() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let first = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        611,
    );
    let second = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        612,
    );
    assert!(first.index() < second.index());
    let first_creation_order = engine.world.original_creation_order(first);
    assert_ne!(first_creation_order, first.index());
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: first_creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: engine.control.frame_counter,
    });

    queue_and_settle_speech(
        &mut engine,
        &assets,
        first,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, first), Some(2));

    queue_and_settle_speech(
        &mut engine,
        &assets,
        second,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, second), None);
    assert!(exclamation_for(&engine, second).is_some());
}

#[test]
fn this_guy_forbid_preserves_original_uword_narrowing_and_ulong_comparison() {
    use std::collections::BTreeMap;

    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        613,
    );
    let creation_order = u32::from(u16::MAX) + 2;
    engine.world.install_original_creation_orders(
        BTreeMap::from([(owner, creation_order)]),
        creation_order + 1,
    );
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Drunken,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: engine.control.frame_counter,
    });

    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Drunken,
        SpeechFlags::empty(),
    );

    assert_ne!(last_speech_impossible(&engine, owner), Some(2));
    let personal = engine
        .ai
        .global
        .forbidden_remarks
        .last()
        .expect("accepted Drunken speech adds its personal forbid");
    assert_eq!(personal.flags, RemarkTargetFlags::THIS_GUY.bits());
    assert_eq!(personal.guy_index, creation_order as u16);
}

#[test]
fn missing_speech_profile_is_lazy_for_early_and_non_type_rejections() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let assets = LevelAssets::new();
    let mut early = EngineInner::new();
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!()
    };
    soldier.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(99);
    soldier.element.blipped = true;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = early.add_entity(entity);
    queue_and_settle_speech(
        &mut early,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&early, owner), Some(0));

    early
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .blipped = false;
    let owner_creation_order = early.world.original_creation_order(owner);
    early.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: owner_creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: early.control.frame_counter,
    });
    queue_and_settle_speech(
        &mut early,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&early, owner), Some(2));
}

#[test]
#[should_panic(expected = "speech owner 0 requires missing soldier profile 99 after early gates")]
fn live_this_type_forbid_candidate_requires_contextual_speech_profile() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!()
    };
    soldier.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(99);
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_entity(entity);
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_TYPE.bits(),
        speech_id: 123,
        guy_index: 0,
        bad_guy: true,
        forbidden_till_frame: engine.control.frame_counter,
    });
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
}

#[test]
fn speech_snapshot_roundtrip_and_hash_cover_fifo_live_identity_and_global_state() {
    use crate::ai::{
        AiOwnerWork, AiSpeechAttempt, ForbiddenRemark, Remark, RemarkTargetFlags, ScreenRemark,
        SpeechFlags,
    };

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let first = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        701,
    );
    let second = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        702,
    );
    let first_creation_order = engine.world.original_creation_order(first);
    let ai = engine
        .get_entity_mut(first)
        .unwrap()
        .ai_controller_mut()
        .unwrap();
    ai.current_remark = Remark::Arrow;
    ai.current_remark_flags = SpeechFlags::MYTALK_2.bits();
    ai.outbox.reentrant.owner_work = vec![
        AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::Arrow,
            flags: 0,
        }),
        AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::WaspSting,
            flags: SpeechFlags::ALWAYS.bits(),
        }),
    ];
    engine
        .feedback
        .sound_sim
        .playing_exclamations
        .push(crate::sound::PlayingExclamation {
            actor_id: first.index(),
            exclamation_id: Remark::Arrow as u32,
            finish_frame: 77,
        });
    engine.ai.global.current_speech_variant = 2;
    engine.ai.global.screen_remarks.push(ScreenRemark {
        timer: 100,
        prefix: "snapshot".into(),
        remark: Remark::Arrow,
    });
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 701,
        guy_index: first_creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: 88,
    });

    let json = serde_json::to_string(&engine).expect("serialize speech snapshot");
    let restored: EngineInner = serde_json::from_str(&json).expect("deserialize speech snapshot");
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&engine)
    );
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&engine).unwrap()
    );

    let mut reordered = engine.clone();
    reordered
        .get_entity_mut(first)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .reentrant
        .owner_work
        .reverse();
    assert_ne!(
        robin_util::state_hash::compute(&reordered),
        robin_util::state_hash::compute(&engine)
    );

    let mut retargeted = engine.clone();
    retargeted.feedback.sound_sim.playing_exclamations[0].actor_id = second.index();
    assert_ne!(
        robin_util::state_hash::compute(&retargeted),
        robin_util::state_hash::compute(&engine)
    );
}

#[test]
fn specialized_ai_continuation_snapshot_roundtrip_and_hash_cover_pending_barrier() {
    use crate::ai::{
        AlertContinuation, AlertSoldiersFailureContinuation, CrossNpcAction, Position,
        StimulusInfo, StimulusType, ThinkResultContinuation,
    };

    let mut engine = EngineInner::new();
    let caller = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    engine
        .get_entity_mut(caller)
        .and_then(Entity::ai_controller_mut)
        .expect("snapshot caller has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .extend([
            CrossNpcAction::RequestAlert {
                target: target.index(),
                caller: caller.index(),
                continuation: AlertContinuation::SoldierSawOfficer,
            },
            CrossNpcAction::RequestThinkResult {
                target: target.index(),
                caller: caller.index(),
                stimulus_type: StimulusType::CallAlert,
                info: StimulusInfo::Human(target.index()),
                continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                    last: true,
                    use_formation: true,
                    failure: AlertSoldiersFailureContinuation::SeekBody {
                        center: Position {
                            x: 8.0,
                            y: 16.0,
                            ..Default::default()
                        },
                        radius: 160,
                    },
                },
            },
            CrossNpcAction::FinalizeAlertSoldiers {
                caller: caller.index(),
                use_formation: true,
                failure: AlertSoldiersFailureContinuation::SeekBody {
                    center: Position {
                        x: target.index() as f32 + 12.5,
                        y: -7.0,
                        ..Default::default()
                    },
                    radius: 320,
                },
            },
        ]);

    let json = serde_json::to_string(&engine).expect("serialize AI continuation snapshot");
    let restored: EngineInner =
        serde_json::from_str(&json).expect("deserialize AI continuation snapshot");
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&engine).unwrap()
    );
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&engine)
    );

    let mut changed_continuation_payload = engine.clone();
    let actions = &mut changed_continuation_payload
        .get_entity_mut(caller)
        .and_then(Entity::ai_controller_mut)
        .expect("snapshot caller retains AI")
        .outbox
        .reentrant
        .cross_npc_actions;
    let CrossNpcAction::RequestThinkResult {
        continuation: ThinkResultContinuation::OfficerAlertedSoldier { last, .. },
        ..
    } = &mut actions[1]
    else {
        panic!("snapshot test lost its result-bearing continuation")
    };
    *last = false;
    assert_ne!(
        robin_util::state_hash::compute(&changed_continuation_payload),
        robin_util::state_hash::compute(&engine),
        "the nested continuation payload must participate in the deterministic hash"
    );
}

/// Build a minimal civilian entity for NPC-translate tests.
pub(super) fn make_test_civilian(posture: crate::element::Posture) -> Entity {
    Entity::Civilian(crate::element::ActorCivilian {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorCivilian,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: Default::default(),
    })
}

#[test]
fn alert_soldier_typed_tail_owns_couldnt_reachpoint_before_event_surface() {
    use crate::element::AiBrain;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(owner)
        .expect("AlertSoldier test civilian exists")
    else {
        panic!("AlertSoldier test owner changed kind")
    };
    civilian.npc.ai_brain =
        AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(owner.index())));
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("AlertSoldier test civilian has AI");
    ai.completion_latch_inside_think = true;
    ai.couldnt_reachpoint = true;
    ai.outbox.reentrant.alert_soldier_completion_pending = true;

    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AlertSoldier test civilian retains AI");
    assert!(ai.couldnt_reachpoint);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());

    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AlertSoldier test civilian retains mutable AI")
        .outbox
        .reentrant
        .alert_soldier_completion_pending = false;
    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AlertSoldier test civilian retains AI after surface");
    assert!(!ai.couldnt_reachpoint);
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        vec![crate::ai::StimulusType::EventCouldntReachPoint]
    );
}

#[test]
fn tower_guard_alert_officer_tail_consumes_ignored_route_failure() {
    use crate::ai::{AiOwnerWork, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("tower-guard call-me owner has Enemy AI");
        ai.base.completion_latch_inside_think = true;
        ai.base.couldnt_reachpoint = true;
        ai.base
            .outbox
            .reentrant
            .tower_guard_alert_officer_completion_pending = true;
        ai.base
            .outbox
            .reentrant
            .owner_work
            .push(AiOwnerWork::ConsumeTowerGuardAlertOfficerRouteFailure);
    }

    // The generic EndThink surface must leave AlertOfficer's synchronous
    // result for its typed no-result tail, rather than dispatching a seek.
    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("tower-guard call-me owner retains Enemy AI");
    assert!(ai.base.couldnt_reachpoint);
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());

    engine.drain_ai_owner_work_for(&sim, &assets, owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("tower-guard call-me owner retains Enemy AI after tail");
    assert!(!ai.base.couldnt_reachpoint);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .tower_guard_alert_officer_completion_pending
    );
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(
        !ai.base
            .outbox
            .reentrant
            .self_stimuli
            .iter()
            .any(|queued| queued.stimulus_type == StimulusType::EventCouldntReachPoint)
    );
}

#[test]
fn dead_body_alert_tail_consumes_route_failure_before_generic_event_surface() {
    use crate::ai::{AiOwnerWork, StimulusType};
    use crate::ai_enemy::SeekFlags;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "dead_body_alert_continuation_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission supports owner-local movement settlement"),
    );
    let center = crate::ai::Position {
        x: 175.0,
        y: 225.0,
        ..Default::default()
    };
    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("dead-body-alert owner has Enemy AI");
        ai.base.completion_latch_inside_think = true;
        ai.base.couldnt_reachpoint = true;
        ai.base.outbox.reentrant.dead_body_alert_completion_pending = true;
        ai.base.outbox.reentrant.owner_work.push(
            AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer {
                center,
                radius: 300,
            },
        );
    }

    // EndThink observes the typed latch before the owner continuation runs.
    engine.surface_synchronous_completion_events_for_owner(owner);
    engine.drain_ai_owner_work_for(&sim, &assets, owner);
    engine.drain_direct_ai_owner_boundary_mode(&sim, owner, &assets, true, true);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("dead-body-alert owner retains Enemy AI");
    assert!(!ai.base.couldnt_reachpoint);
    assert!(!ai.base.outbox.reentrant.dead_body_alert_completion_pending);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    assert!(
        !ai.base
            .outbox
            .reentrant
            .self_stimuli
            .iter()
            .any(|queued| queued.stimulus_type == StimulusType::EventCouldntReachPoint)
    );
    assert_eq!(
        ai.seek_flags,
        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
    );
    assert_eq!(
        ai.personal_seek_point_2
            .as_ref()
            .expect("failed officer route creates the personal endpoint")
            .position,
        center
    );
}

#[test]
fn unrelated_running_to_officer_failure_remains_generic() {
    use crate::ai::{AiState, StimulusType, Substate};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
#[should_panic(expected = "non-enemy AI brain")]
fn dead_body_alert_tail_fails_loud_for_wrong_ai_owner() {
    use crate::ai::{AiOwnerWork, Position};
    use crate::element::AiBrain;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(owner)
        .expect("wrong-kind continuation owner exists")
    else {
        unreachable!()
    };
    soldier.npc.ai_brain =
        AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(owner.index())));
    soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("friendly AI has base")
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer {
            center: Position::default(),
            radius: 300,
        });
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_ai_owner_work_for(&sim, &assets, owner);
}

fn make_alert_soldier_owner(engine: &mut EngineInner) -> EntityId {
    use crate::element::AiBrain;

    let owner = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(owner)
        .expect("AlertSoldier test civilian exists")
    else {
        panic!("AlertSoldier test owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    civilian.civilian.cached_camp = crate::element::Camp::Lacklandists;
    civilian.npc.ai_brain =
        AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(owner.index())));
    owner
}

#[test]
fn alert_soldier_owner_boundary_first_route_success_runs_success_tail() {
    use crate::ai::{AiOwnerWork, Remark};
    use crate::ai_friendly::AlertSoldierFailureContinuation;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = make_alert_soldier_owner(&mut engine);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AlertSoldier owner has AI");
    ai.outbox.reentrant.alert_soldier_completion_pending = true;
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeFriendlyAlertSoldierAfterGoNear {
            center: Default::default(),
            check_door_path: false,
            failure: AlertSoldierFailureContinuation::Panic,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AlertSoldier owner retains AI");
    assert_eq!(ai.current_remark, Remark::CivPanic);
    assert!(!ai.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn alert_soldier_owner_boundary_first_failure_retries_and_consumes_success() {
    use crate::ai::{AiOwnerWork, Remark};
    use crate::ai_friendly::AlertSoldierFailureContinuation;
    use crate::coordinates::MapPoint;
    use crate::element::Camp;
    use crate::position_interface::SectorHandle;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = make_alert_soldier_owner(&mut engine);
    let soldier = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(owner, 0.0), (soldier, 100.0)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("AlertSoldier route actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
        entity.element_data_mut().set_sector(SectorHandle::new(1));
        entity.element_data_mut().set_layer(0);
        entity
            .npc_data_mut()
            .expect("route actor is NPC")
            .life_points = 100;
    }
    engine.ai.global.all_soldier_handles = std::sync::Arc::new(vec![soldier.index()]);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AlertSoldier owner has AI");
    ai.couldnt_reachpoint = true;
    ai.completion_latch_inside_think = true;
    ai.outbox.reentrant.alert_soldier_completion_pending = true;
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeFriendlyAlertSoldierAfterGoNear {
            center: Default::default(),
            check_door_path: false,
            failure: AlertSoldierFailureContinuation::Panic,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AlertSoldier owner retains AI");
    assert_eq!(ai.antagonist, soldier.index());
    assert_eq!(ai.current_remark, Remark::CivPanic);
    assert!(!ai.couldnt_reachpoint);
    assert!(!ai.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.outbox.actor.begin_panic.is_none());
}

#[test]
fn alert_soldier_owner_boundary_second_failure_runs_typed_tail_without_event4() {
    use crate::ai::{AiOwnerWork, AiState};
    use crate::ai_friendly::AlertSoldierFailureContinuation;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = make_alert_soldier_owner(&mut engine);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AlertSoldier owner has AI");
    ai.couldnt_reachpoint = true;
    ai.completion_latch_inside_think = true;
    ai.outbox.reentrant.alert_soldier_completion_pending = true;
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeFriendlyAlertSoldierAfterGoNear {
            center: Default::default(),
            check_door_path: false,
            failure: AlertSoldierFailureContinuation::Panic,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AlertSoldier owner retains AI");
    assert_eq!(ai.current_state, AiState::Fleeing);
    assert!(!ai.couldnt_reachpoint);
    assert!(!ai.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

pub(super) fn make_test_pc(posture: crate::element::Posture) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    })
}

#[test]
fn alert_soldier_friend_append_drain_preserves_preexisting_duplicate_and_order() {
    use crate::element::{AiBrain, Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let first_friend = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let second_friend = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("alerting civilian exists")
    else {
        panic!("alerting civilian changed kind")
    };
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        civilian_id.index(),
    )));
    civilian.npc.detectable_lists[DetectableType::Friend as usize].push(Detectable {
        element: Some(first_friend),
        detectable_type: DetectableType::Friend,
        ..Default::default()
    });
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("alerting civilian has AI");
    ai.owner_entity_id = Some(civilian_id);
    // This is the exact duplicate-preserving lane used by AlertSoldier's
    // direct retail AddDetectable calls.
    ai.outbox.actor.append_detectables.extend([
        (first_friend, DetectableType::Friend),
        (second_friend, DetectableType::Friend),
    ]);

    engine.drain_pending_for_npc(&sim, civilian_id, &LevelAssets::default());

    let friends = &engine
        .get_entity(civilian_id)
        .expect("alerting civilian remains live")
        .npc_data()
        .expect("alerting civilian retains NPC data")
        .detectable_lists[DetectableType::Friend as usize];
    assert_eq!(
        friends
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(first_friend), Some(first_friend), Some(second_friend)],
        "the existing friend must be duplicated and the AlertSoldier registry order retained"
    );
}

#[test]
fn original_pc_registry_is_independent_from_portrait_priority_order() {
    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    // Authoritative topology can differ from Rust's provisional construction
    // slots. Installing it establishes Original AddElement order once.
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([(first, 101), (second, 100)]),
        102,
    );
    assert_eq!(engine.world.original_pc_registry_ids, vec![second, first]);

    // Portrait sorting is a separate UI concern and must not mutate the
    // engine registry used by GetPC/marrayActorsPC gameplay loops.
    engine.world.pc_ids = vec![first, second];
    assert_eq!(engine.world.original_pc_registry_ids, vec![second, first]);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    assert_eq!(
        engine.ai_pc_snapshot_ids_for_test(&assets),
        vec![second, first],
        "AI snapshots must scan Original's registry, not portrait priority"
    );

    engine.remove_entity(second);
    assert_eq!(engine.world.pc_ids, vec![first]);
    assert_eq!(engine.world.original_pc_registry_ids, vec![first]);
}

/// Give the default (empty) test grid a real map bounding box.
///
/// Position authorization rejects boxes wholly outside the level's map
/// bbox, and a default-constructed grid has no bbox at all — every
/// placement query fails. Tests that exercise formation placement need
/// an open field instead.
pub(super) fn install_test_open_field_bbox(engine: &mut EngineInner) {
    let mut level = (*engine.world.fast_grid.level).clone();
    level.map_bbox = MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
}

pub(super) fn install_test_building_sector(engine: &mut EngineInner, raw_sector: u16) {
    let _sector = crate::position_interface::SectorHandle::new(raw_sector)
        .expect("test building sector must be non-zero");
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(raw_sector as i16), 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: MapBBox::new(),
        sector_type: crate::sector::SectorType::BUILDING,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(raw_sector as i16),
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
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
}

#[test]
fn selection_mark_skips_hidden_and_building_pcs() {
    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
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
fn enter_swordfight_clears_pending_bow_shot_list() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(pc),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    assert!(engine.pc_has_pending_shoot_bow(pc));

    let _ = engine.enter_swordfight(sim, &assets, pc, opponent, false);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Interrupted
    );
    assert!(
        !engine.pc_has_pending_shoot_bow(pc),
        "C++ EnterSwordFight clears the actor's pending shoot list before validity checks"
    );
}

#[test]
fn npc_enter_swordfight_preserves_postponed_bow_sequence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let initiator = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(initiator),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    engine.orders.sequence_manager.postpone_element(shot_seq, 0);

    let _ = engine.enter_swordfight(sim, &assets, initiator, opponent, false);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Postponed,
        "Original ClearShootList removes the retained NPC pointer without interrupting its sequence"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .drain_pending_condolations()
            .is_empty(),
        "clearing an NPC shoot pointer must not invent EventDone"
    );
}

#[test]
fn reciprocal_swordfight_entry_preserves_existing_opponent_strength() {
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let initiator = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .get_entity_mut(initiator)
        .and_then(Entity::human_data_mut)
        .unwrap()
        .relative_fighting_ability = 17;
    {
        let human = engine
            .get_entity_mut(opponent)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![initiator];
        human.relative_fighting_ability = 42;
    }

    assert!(engine.enter_swordfight(&sim, &assets, initiator, opponent, false));

    let initiator_human = engine
        .get_entity(initiator)
        .and_then(Entity::human_data)
        .unwrap();
    assert_eq!(initiator_human.opponents, vec![opponent]);
    assert_eq!(initiator_human.relative_fighting_ability, 50);

    let opponent_human = engine
        .get_entity(opponent)
        .and_then(Entity::human_data)
        .unwrap();
    assert_eq!(opponent_human.opponents, vec![initiator]);
    assert_eq!(opponent_human.relative_fighting_ability, 42);
}

#[test]
fn far_opponent_removal_retains_owner_strength_and_runs_reciprocal_delete() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();

    let owner = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let near = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let mut far_entity = make_test_pc(crate::element::Posture::Upright);
    far_entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_000.0, 0.0, 0.0));
    far_entity
        .element_data_mut()
        .set_position_map(MapPoint::new(1_000.0, 0.0));
    let far = engine.add_entity(far_entity);
    let mut far_partner_entity = make_test_soldier(crate::element::Posture::Upright);
    far_partner_entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_000.0, 0.0, 0.0));
    far_partner_entity
        .element_data_mut()
        .set_position_map(MapPoint::new(1_000.0, 0.0));
    let far_partner = engine.add_entity(far_partner_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    {
        let human = engine
            .get_entity_mut(owner)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![near, far];
        human.relative_fighting_ability = 17;
    }
    engine
        .get_entity_mut(near)
        .and_then(Entity::human_data_mut)
        .unwrap()
        .opponents = vec![owner];
    {
        let human = engine
            .get_entity_mut(far)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![owner, far_partner];
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }
    {
        let human = engine
            .get_entity_mut(far_partner)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![far];
        human.smalltalk_initiative = true;
    }

    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);

    let owner_human = engine.get_entity(owner).unwrap().human_data().unwrap();
    assert_eq!(owner_human.opponents, vec![near]);
    assert_eq!(owner_human.relative_fighting_ability, 17);

    let far_human = engine.get_entity(far).unwrap().human_data().unwrap();
    assert_eq!(far_human.opponents, vec![far_partner]);
    assert!(far_human.smalltalk_initiative);
    assert!(far_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(far_partner)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative
    );
}

#[test]
fn terminal_sword_provoke_observes_promoted_opponent_before_post_seek_speak() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Command;
    use crate::sequence::{Sequence, SequenceElement};

    // Linux3/Profile001/Savegame_014/replay-040, frame 18433: the terminal
    // step puts the old principal just beyond UBER while a reciprocal second
    // opponent remains between MAXIMAL and UBER. Original removes/promotes
    // inside Human::Execute, registers Provoke, and only then registers the
    // point-seek SpeakHeroReachDestination tail.
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let old_principal = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let promoted = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let weapon = profiles
        .hth_weapons
        .first_mut()
        .expect("complete actor fixture supplies an HtH weapon");
    weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 90;
    weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 150;

    let positions = [
        (owner, 151.583_5_f32),
        (old_principal, 0.0_f32),
        (promoted, 18.567_36_f32),
    ];
    for (entity_id, x) in positions {
        let entity = engine.get_entity_mut(entity_id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![old_principal, promoted];
    for opponent in [old_principal, promoted] {
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![owner];
    }

    assert!(
        !engine.sword_movement_termination_warrants_provoke(&assets, owner),
        "the >UBER old principal must make a pre-removal snapshot false"
    );
    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![promoted]
    );
    assert!(
        engine.sword_movement_termination_warrants_provoke(&assets, owner),
        "the promoted reciprocal opponent is inside the Provoke band"
    );

    engine.launch_sword_movement_termination_provoke(owner);
    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new(
        2,
        Command::SpeakHeroReachDestination,
        Some(owner),
    ));
    engine.launch_sequence(post_seek);
    let owner_registrations = engine
        .orders
        .sequence_manager
        .v48_elements_to_go()
        .into_iter()
        .filter_map(|(sequence_id, element_index)| {
            engine
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
        })
        .filter(|element| element.owner == Some(owner))
        .map(|element| element.command)
        .collect::<Vec<_>>();
    assert_eq!(
        owner_registrations,
        vec![Command::Provoke, Command::SpeakHeroReachDestination],
        "terminal Execute must register exactly one Provoke before post-seek Speak"
    );
}

#[test]
fn sword_movement_start_gives_initiative_to_principal_promoted_by_far_pruning() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    // Human::Execute performs QuitSwordfightWithFarOpponents immediately
    // after PerformMotion and only then handles RHMOTION_START. The old
    // principal can therefore disappear before the START initiative handoff.
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let old_principal = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let promoted = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let weapon = profiles
        .hth_weapons
        .first_mut()
        .expect("complete actor fixture supplies an HtH weapon");
    weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 150;

    for (entity_id, x) in [
        (owner, 151.583_5_f32),
        (old_principal, 0.0_f32),
        (promoted, 18.567_36_f32),
    ] {
        let entity = engine.get_entity_mut(entity_id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![old_principal, promoted];
    for opponent in [old_principal, promoted] {
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![owner];
    }

    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);
    engine.apply_sword_movement_start_initiative_transfer(owner);

    let owner_human = engine.get_entity(owner).unwrap().human_data().unwrap();
    assert_eq!(owner_human.opponents, vec![promoted]);
    assert!(!owner_human.smalltalk_initiative);
    let promoted_human = engine.get_entity(promoted).unwrap().human_data().unwrap();
    assert!(promoted_human.smalltalk_initiative);
    assert!(promoted_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(old_principal)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative,
        "the pruned old principal must not receive the START handoff"
    );
}

pub(super) fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    entity
}

#[test]
fn direct_ai_owner_boundary_preserves_preexisting_foreign_condolation() {
    use crate::element::Command;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let foreign_a = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let foreign_b = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let terminate = |engine: &mut EngineInner, card_owner| {
        let sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::Wait, Some(card_owner)));
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .orders
            .sequence_manager
            .element_terminated(sequence, 0);
    };
    terminate(&mut engine, foreign_a);
    terminate(&mut engine, owner);
    terminate(&mut engine, foreign_b);

    // Exercise the nested global drain in `drain_pending_for_npc_mode`, not
    // merely the idle direct-boundary endpoint. The owner's Halt and its
    // pre-existing root must close now, while foreign A/B stay queued.
    let live_owner_sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Wait, Some(owner)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(live_owner_sequence, 0);
    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("direct-boundary owner has AI")
        .outbox
        .actor
        .halt = true;

    let ((), stimuli) = crate::engine::soldier_helpers::capture_condolation_stimuli(|| {
        engine.drain_direct_ai_owner_boundary_without_forecast(&sim, owner, &assets);
    });

    let backlog = engine.orders.sequence_manager.drain_pending_condolations();
    assert_eq!(
        backlog
            .iter()
            .map(|dispatch| dispatch.card.owner)
            .collect::<Vec<_>>(),
        vec![foreign_a, foreign_b],
        "pre-existing foreign cards retain their FIFO around owner Halt recursion"
    );
    assert!(
        stimuli.iter().any(|(stimulus_owner, stimulus)| {
            *stimulus_owner == owner && *stimulus == crate::ai::StimulusType::EventDone
        }),
        "the owner's pre-existing terminal root must be delivered, not dropped"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(owner, |_| true),
        "the selected owner's Halt must still close causally inside the direct boundary"
    );
}

#[test]
fn synchronous_one_shot_noise_is_handled_before_broadcast_returns() {
    use crate::ai::{AiState, NoiseType, Stimulus, StimulusType, Substate};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(crate::profiles::SoldierProfile::default());

    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(WorldPoint3D::new(10.0, 10.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(10.0, 10.0));
    let listener_id = engine.add_entity(listener);
    engine
        .get_entity_mut(listener_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("test listener has enemy AI")
        .base
        .me = listener_id.index();
    engine
        .get_entity_mut(listener_id)
        .and_then(Entity::ai_controller_mut)
        .expect("test listener has base AI")
        .outbox
        .detection
        .stimuli
        .push(Stimulus::new(StimulusType::EventTimer));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.broadcast_noise_synchronously(
        &sim,
        &assets,
        NoiseType::Bonk,
        MapPoint::new(20.0, 10.0),
        0,
        crate::parameters_ai::NOISE_VOLUME_BONK as u16,
        0,
        None,
    );

    let listener = engine
        .get_entity(listener_id)
        .and_then(Entity::enemy_ai)
        .expect("test listener survives synchronous noise");
    assert_eq!(
        listener
            .base
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventTimer],
        "direct EVENT_HEAR must not consume an unrelated deferred FIFO"
    );
    assert_eq!(listener.base.current_state, AiState::Wondering);
    assert_eq!(listener.base.current_substate, Substate::WonderingWatching);
}

#[test]
fn one_shot_noise_listener_walk_uses_restored_original_creation_order() {
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let second = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_order = engine.world.original_creation_order(first);
    let second_order = engine.world.original_creation_order(second);
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([(first, second_order), (second, first_order)]),
        second_order + 1,
    );

    assert_eq!(engine.one_shot_noise_listener_ids(), vec![second, first]);
}

#[test]
fn one_shot_hearing_defers_listener_state_filtering_but_rejects_its_source_point() {
    use crate::ai::NoiseType;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 5;
    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier.element.active = false;
    soldier.human.unconscious = true;
    soldier
        .element
        .set_position(WorldPoint3D::new(10.0, 10.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(10.0, 10.0));
    let listener_id = engine.add_entity(listener);

    let audible = engine.one_shot_noise(
        NoiseType::Bonk,
        MapPoint::new(20.0, 10.0),
        0,
        crate::parameters_ai::NOISE_VOLUME_BONK as u16,
        0,
        None,
    );
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, audible)
            .is_some(),
        "inactive/unconscious state belongs to StartThink, after GetHearVolume"
    );
    assert_eq!(
        engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener keeps NPC state")
            .old_cover_noise_deafness_frame_counter,
        5,
        "GetHearVolume must refresh deafness before StartThink refuses the event"
    );

    let same_point = engine.one_shot_noise(
        NoiseType::Aaargh,
        MapPoint::new(10.0, 10.0),
        0,
        crate::parameters_ai::NOISE_VOLUME_AAARGH as u16,
        0,
        Some(listener_id),
    );
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, same_point)
            .is_none(),
        "the actor at the exact full-3D source point must not hear its own cry"
    );

    engine.control.frame_counter = 6;
    let max_norm_only =
        engine.one_shot_noise(NoiseType::Bonk, MapPoint::new(18.0, 14.0), 0, 10, 0, None);
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, max_norm_only)
            .is_none(),
        "a source inside the max-norm box can still have no positive Euclidean remainder"
    );
    assert_eq!(
        engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener keeps NPC state")
            .old_cover_noise_deafness_frame_counter,
        5,
        "GetHearVolume must not refresh deafness until subjective volume is positive"
    );
}

#[test]
fn one_shot_hearing_uses_authoritative_world_y_at_uword_volume_boundary() {
    use crate::ai::NoiseType;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;
    use crate::position_interface::INVERSE_ASPECT_RATIO;

    // Find the adjacent f32 world-Y values whose aspect-stretched distances
    // straddle 499. Original truncates `500 - distance` to UWORD, so the
    // lower reconstructed value is audible at volume 1 while the upper
    // authoritative value is inaudible at volume 0.
    let boundary = 499.0_f32 / INVERSE_ASPECT_RATIO;
    let mut reconstructed_y = boundary;
    while reconstructed_y * INVERSE_ASPECT_RATIO >= 499.0 {
        reconstructed_y = f32::from_bits(reconstructed_y.to_bits() - 1);
    }
    let mut authoritative_y = boundary;
    while authoritative_y * INVERSE_ASPECT_RATIO <= 499.0 {
        authoritative_y = f32::from_bits(authoritative_y.to_bits() + 1);
    }
    assert_eq!((500.0 - reconstructed_y * INVERSE_ASPECT_RATIO) as u16, 1);
    assert_eq!((500.0 - authoritative_y * INVERSE_ASPECT_RATIO) as u16, 0);

    let mut engine = EngineInner::new();
    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier
        .element
        .set_position(WorldPoint3D::new(0.0, authoritative_y, 0.0));
    // Projection roundoff can make map Y reconstruct to the adjacent lower
    // float even though stored world Y remains authoritative.
    soldier
        .element
        .set_position_map_preserving_3d(MapPoint::new(0.0, reconstructed_y));
    let listener_id = engine.add_entity(listener);
    let drawbridge = engine.one_shot_noise(
        NoiseType::Drawbridge,
        MapPoint::new(0.0, 0.0),
        0,
        crate::parameters_ai::NOISE_VOLUME_DRAWBRIDGE as u16,
        0,
        None,
    );

    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, drawbridge)
            .is_none(),
        "GetHearVolume must use stored 3D Y; reconstructing map Y would create a spurious volume-1 listener"
    );
}

#[test]
fn soldier_death_detaches_guard_and_archery_before_forcing_quiet_music() {
    use crate::ai::{
        AiState, AlertLevel, ArcheryReservationRelease, GuardedPcEffect, PointArchery,
        ReservedShootingPoint, SectorArchery, Substate,
    };
    use crate::entity_id::PcId;
    use crate::sector::{ArcheryPointIdx, SectorNumber};
    use crate::sound::MusicMode;

    let mut engine = EngineInner::new();

    let old_guarded_pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let current_guarded_pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let EntityId::Pc(old_guarded_pc_typed) = old_guarded_pc else {
        panic!("test PC has a non-PC entity ID")
    };
    let EntityId::Pc(current_guarded_pc_typed) = current_guarded_pc else {
        panic!("test PC has a non-PC entity ID")
    };

    let mut victim = make_test_ai_soldier(crate::element::Camp::Lacklandists);
    let Entity::Soldier(victim_soldier) = &mut victim else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    victim_soldier.element.active = true;
    victim_soldier.npc.life_points = 100;
    let victim_id = engine.add_entity(victim);

    for guarded_pc in [old_guarded_pc, current_guarded_pc] {
        let Some(Entity::Pc(pc)) = engine.get_entity_mut(guarded_pc) else {
            panic!("test guarded PC exists")
        };
        pc.element.active = true;
        pc.pc.life_points = 100;
        pc.pc.guard = Some(victim_id);
    }

    engine.ai.global.archery_sectors.push(SectorArchery {
        points: vec![PointArchery {
            position: Default::default(),
            direction: 0,
            is_shooting_point: true,
            sector_index: SectorNumber::new(1),
            owner: Some(victim_id),
        }],
        polygon: Vec::new(),
        layer: 0,
        index_first_shooting_point: Some(ArcheryPointIdx(0)),
        index_last_shooting_point: Some(ArcheryPointIdx(0)),
        num_shooting_points: 1,
        num_owners: 1,
    });

    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity_mut(victim_id) else {
        panic!("test victim exists")
    };
    let enemy = victim_soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test victim has enemy AI");
    enemy.guarded_pc = Some(PcId(current_guarded_pc_typed.0));
    enemy.base.outbox.actor.set_guarded_pc = Some(GuardedPcEffect {
        old: Some(PcId(old_guarded_pc_typed.0)),
        new: Some(PcId(current_guarded_pc_typed.0)),
    });
    // Model SetState having synchronously cleared the AI-side shooting
    // point while its reciprocal/global release is still queued.
    enemy.my_shooting_point = None;
    enemy.my_archery_sector = Some(0);
    enemy.base.outbox.actor.archery_reservation_release = ArcheryReservationRelease {
        shooting_point: Some(ReservedShootingPoint {
            sector_index: 0,
            point_index: ArcheryPointIdx(0),
        }),
        release_sector: true,
    };
    enemy.base.current_state = AiState::Menacing;
    enemy.base.current_substate = Substate::MenacingPcInComa;
    enemy.base.current_music_alert_status = AlertLevel::Red;
    enemy.base.view_alert_status = AlertLevel::Red;
    enemy.base.outbox.actor.halt = true;
    engine.ai.global.overall_villain_alert_status = AlertLevel::Red;
    engine.ai.global.overall_alert_status = AlertLevel::Red;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim_id);

    for guarded_pc in [old_guarded_pc, current_guarded_pc] {
        let Some(Entity::Pc(pc)) = engine.get_entity(guarded_pc) else {
            panic!("test guarded PC survives")
        };
        assert_eq!(pc.pc.guard, None);
    }
    assert_eq!(engine.ai.global.archery_sectors[0].points[0].owner, None);
    assert_eq!(engine.ai.global.archery_sectors[0].num_owners, 0);

    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity(victim_id) else {
        panic!("test victim survives as a corpse")
    };
    let enemy = victim_soldier
        .npc
        .ai_brain
        .enemy()
        .expect("test victim retains enemy AI");
    assert_eq!(enemy.guarded_pc, None);
    assert_eq!(enemy.my_archery_sector, None);
    assert_eq!(enemy.base.current_state, AiState::Sleeping);
    assert_eq!(enemy.base.current_substate, Substate::SleepingForever);
    assert!(!enemy.base.outbox.actor.halt);
    assert_eq!(
        enemy.base.outbox.actor.archery_reservation_release,
        ArcheryReservationRelease::default()
    );
    assert!(enemy.base.outbox.music.instant_change);

    engine.update_overall_villain_alert(&assets.profile_manager);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .sounds
            .iter()
            .any(|command| matches!(command, SoundCommand::ForceMusicMode(MusicMode::Quiet)))
    );
}

#[test]
fn soldier_death_detaches_both_combat_neighbours_without_touching_another_line() {
    use crate::entity_id::SoldierId;

    let mut engine = EngineInner::new();
    // Reserve handle zero, which EnemyAi uses as its null neighbour sentinel.
    // This models Lane 36's Soldier 90 — dying Soldier 91 — Soldier 54 line.
    let _sentinel = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let left = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let right = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unrelated_left =
        engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unrelated_right =
        engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    let left_handle = left.index();
    let victim_handle = victim.index();
    let right_handle = right.index();
    let unrelated_left_handle = unrelated_left.index();
    let unrelated_right_handle = unrelated_right.index();
    for (entity, left_neighbour, right_neighbour) in [
        (left, 0, victim_handle),
        (victim, left_handle, right_handle),
        (right, victim_handle, 0),
        (unrelated_left, 0, unrelated_right_handle),
        (unrelated_right, unrelated_left_handle, 0),
    ] {
        let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(entity) else {
            panic!("combat-neighbour fixture contains a non-soldier")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        enemy.left_combat_neighbour = left_neighbour;
        enemy.right_combat_neighbour = right_neighbour;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim);

    let links = |engine: &EngineInner, handle: u32| {
        let Some(Entity::Soldier(soldier)) = engine
            .world
            .entities
            .get(EntityId::Soldier(SoldierId(handle)))
        else {
            panic!("combat-neighbour fixture soldier disappeared")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("test soldier retains enemy AI");
        (enemy.left_combat_neighbour, enemy.right_combat_neighbour)
    };

    assert_eq!(links(&engine, left_handle), (0, 0));
    assert_eq!(links(&engine, victim_handle), (0, 0));
    assert_eq!(links(&engine, right_handle), (0, 0));
    assert_eq!(
        links(&engine, unrelated_left_handle),
        (0, unrelated_right_handle)
    );
    assert_eq!(
        links(&engine, unrelated_right_handle),
        (unrelated_left_handle, 0)
    );
}

#[test]
fn soldier_death_applies_queued_reciprocal_combat_neighbour_clears() {
    use crate::ai::CrossNpcAction;
    use crate::entity_id::SoldierId;

    let mut engine = EngineInner::new();
    let _sentinel = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_left = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_right = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim_handle = victim.index();
    let old_left_handle = old_left.index();
    let old_right_handle = old_right.index();

    for (entity, left_neighbour, right_neighbour) in
        [(old_left, 0, victim_handle), (old_right, victim_handle, 0)]
    {
        let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(entity) else {
            panic!("queued combat-neighbour fixture contains a non-soldier")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        enemy.left_combat_neighbour = left_neighbour;
        enemy.right_combat_neighbour = right_neighbour;
    }
    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity_mut(victim) else {
        panic!("queued combat-neighbour victim is not a soldier")
    };
    let victim_enemy = victim_soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test victim has enemy AI");
    assert_eq!(victim_enemy.left_combat_neighbour, 0);
    assert_eq!(victim_enemy.right_combat_neighbour, 0);
    victim_enemy
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .extend([
            CrossNpcAction::SetRightCombatNeighbour {
                target: old_left_handle,
                neighbour: 0,
            },
            CrossNpcAction::SetLeftCombatNeighbour {
                target: old_right_handle,
                neighbour: 0,
            },
        ]);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim);

    let links = |engine: &EngineInner, handle: u32| {
        let Some(Entity::Soldier(soldier)) = engine
            .world
            .entities
            .get(EntityId::Soldier(SoldierId(handle)))
        else {
            panic!("queued combat-neighbour fixture soldier disappeared")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("test soldier retains enemy AI");
        (enemy.left_combat_neighbour, enemy.right_combat_neighbour)
    };
    assert_eq!(links(&engine, old_left_handle), (0, 0));
    assert_eq!(links(&engine, victim_handle), (0, 0));
    assert_eq!(links(&engine, old_right_handle), (0, 0));
}

#[test]
fn nearby_fighters_keeps_inactive_self_and_filters_ineligible_others() {
    use crate::element::Posture;

    let mut engine = EngineInner::new();
    let self_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let other_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    for id in [self_id, other_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(self_soldier) =
        engine.get_entity_mut(self_id).expect("self fighter exists")
    else {
        panic!("self fighter changed kind")
    };
    self_soldier.element.active = false;

    let Entity::Soldier(other_soldier) = engine
        .get_entity_mut(other_id)
        .expect("other fighter exists")
    else {
        panic!("other fighter changed kind")
    };
    other_soldier.element.posture = Posture::Tied;

    let fighters = engine.build_nearby_fighters_for(
        self_id,
        &assets,
        &crate::sight_obstacle::SharedSightObstacles::default(),
    );
    assert_eq!(fighters.len(), 1);
    assert_eq!(fighters[0].handle, self_id.index());
    assert!(!fighters[0].is_able_to_fight);
    assert!(!fighters[0].is_dead);
    assert!(!fighters[0].is_unconscious);
    assert!(!fighters[0].is_carried);
}

#[test]
fn full_fighter_registry_retains_dead_pc_for_held_ai_targets() {
    let mut engine = EngineInner::new();
    let self_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let dead_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(self_soldier) =
        engine.get_entity_mut(self_id).expect("test fighter exists")
    else {
        panic!("test fighter changed kind")
    };
    self_soldier.element.active = true;
    self_soldier.npc.life_points = 100;
    self_soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test fighter has enemy AI")
        .base
        .me = self_id.index();

    let Entity::Pc(dead_pc) = engine
        .get_entity_mut(dead_pc_id)
        .expect("dead test PC exists")
    else {
        panic!("dead test PC changed kind")
    };
    dead_pc.element.active = true;
    dead_pc.pc.life_points = 0;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let nearby = engine.build_nearby_fighters_for(
        self_id,
        &assets,
        &crate::sight_obstacle::SharedSightObstacles::default(),
    );
    assert!(
        !nearby
            .iter()
            .any(|fighter| fighter.handle == dead_pc_id.index())
    );

    let registry = engine.build_full_fighter_registry_for_test(self_id, &assets);
    let dead_snapshot = registry
        .iter()
        .find(|fighter| fighter.handle == dead_pc_id.index())
        .expect("Original fighter registry retains dead PC objects");
    assert!(dead_snapshot.is_dead);
    assert!(!dead_snapshot.is_able_to_fight);
}

#[test]
fn bow_interaction_accepts_a_target_that_died_while_aiming() {
    use crate::profiles::{BowProfile, BowShootMode, CharacterProfile, ProfileManager};

    let mut engine = EngineInner::new();
    let shooter = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let Entity::Pc(dead_target) = engine.get_entity_mut(target).expect("dead target exists") else {
        panic!("dead target changed kind")
    };
    dead_target.element.active = true;
    dead_target.pc.life_points = 0;

    let mut profiles = ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        shooting_weapon_id: 1,
        shooting: 100,
        ..CharacterProfile::default()
    });
    profiles.bows.push(BowProfile {
        normal_shoot: BowShootMode {
            range: 2000,
            ..BowShootMode::default()
        },
        ..BowProfile::default()
    });
    let mut assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    complete_test_runtime_fixture(&mut engine, &mut assets);

    assert!(engine.shoot_bow_at(&assets, shooter, target).is_some());
}

#[test]
fn friend_swap_candidates_resolve_both_friend_and_target_through_ai_position() {
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

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
            .is_null()
    );
    let friend_ai = friend_soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("friend has AI");
    friend_ai.current_substate = crate::ai::Substate::AttackingRunningToEnemy;
    friend_ai.primary_target = target.index();

    let Entity::Pc(target_pc) = engine.get_entity_mut(target).unwrap() else {
        panic!("target changed kind")
    };
    target_pc
        .element
        .set_position_map(MapPoint::new(21.0, 22.0));
    assert!(target_pc.actor.active_door_pass.is_none());
    // A different live sprite door must not replace the selected movement
    // element's gate or direction for AI Position.
    target_pc
        .element
        .sprite
        .position_iface
        .set_door(crate::position_interface::DoorHandle(2), true);

    for (passing, gate, direction) in [(friend, DoorIndex(0), 1), (target, DoorIndex(1), 0)] {
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
            layer_in: 3,
            ..Door::default()
        },
        Door {
            point_out: MapPoint::new(201.0, 202.0),
            sector_out: SectorNumber::new(22),
            layer_out: 4,
            ..Door::default()
        },
        Door {
            point_in: MapPoint::new(301.0, 302.0),
            sector_in: SectorNumber::new(33),
            layer_in: 5,
            ..Door::default()
        },
    ];

    let candidates = crate::engine::ai::build_friend_swap_candidates(
        &engine.world.entities,
        &engine.script_domains.interactables.doors,
        &engine.orders.sequence_manager,
        owner,
        crate::element::Camp::Lacklandists,
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
    assert_eq!(candidate.friend_primary_target, target.index());
    assert_eq!(candidate.friend_primary_target_position.x, 201.0);
    assert_eq!(candidate.friend_primary_target_position.y, 202.0);
    assert_eq!(
        candidate.friend_primary_target_position.sector,
        crate::position_interface::SectorHandle::new(22)
    );
    assert_eq!(candidate.friend_primary_target_position.level, 4);
}

#[test]
fn ai_position_ignores_misassociated_pass_door_for_non_actor() {
    use crate::coordinates::MapPoint;
    use crate::element::{ElementBonus, ElementData, ElementKind, ObjectData, ObjectType};
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
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
    *gate_id = Some(DoorIndex(0));
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
    use crate::gate::{Door, DoorIndex};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let me_sector = crate::position_interface::SectorHandle::new(1);

    for (id, position) in [
        (owner_id, MapPoint::new(100.0, 0.0)),
        (target_id, MapPoint::new(100.0, 25.0)),
    ] {
        let entity = engine.get_entity_mut(id).expect("roof-wait actor exists");
        entity.element_data_mut().active = true;
        entity.element_data_mut().set_position_map(position);
        entity.element_data_mut().set_sector(me_sector);
    }
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("roof-wait owner has Enemy AI");
    owner.base.me = owner_id.index();
    owner.base.primary_target = target_id.index();

    // The target sprite is still on the owner's side. Original AI Position
    // instead reports point_in/sector_in while this PassDoor is selected.
    let mut door = Door {
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
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
    *gate_id = Some(DoorIndex(0));
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
        &|_| true,
        &|_| None,
    )
    .expect("committed target side exposes the NPC-locked blocking gate");
    assert_eq!(wait.x, 100.0);
    assert_eq!(wait.y, 100.0);
    assert_eq!(wait.sector, me_sector);

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
        &AiContext::default(),
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

    // Without the selected PassDoor, both ordinary live positions are in the
    // same sector. The roof special case must remain absent so the caller can
    // retain couldn't-reachpoint and take its normal emergency fallback.
    engine
        .orders
        .sequence_manager
        .element_terminated(sequence_id, 0);
    assert!(
        crate::engine::ai::precompute_avenger_on_roof_wait_position(
            &engine.world.entities,
            &engine.script_domains.interactables.doors,
            &engine.orders.sequence_manager,
            owner_id,
            target_id,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}

#[test]
fn reconsider_approach_route_settles_before_roof_wait_resume() {
    use crate::ai::{AiContext, AiOwnerWork, AiState, GotoFlags, Position, Substate};
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, GateType};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "reconsider_approach_owner_boundary_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed test jump"),
    );

    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let owner_position = Position {
        x: 0.0,
        y: 0.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let target_position = Position {
        x: 100.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(2),
        level: 1,
    };
    for (id, position) in [(owner_id, owner_position), (target_id, target_position)] {
        let entity = engine.get_entity_mut(id).expect("approach actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    engine
        .get_entity_mut(target_id)
        .and_then(Entity::pc_data_mut)
        .expect("target is a PC")
        .has_jump = true;
    engine.script_domains.interactables.doors = vec![Door {
        gate_type: GateType::Jump,
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        point_out: MapPoint::new(50.0, 100.0),
        point_in: MapPoint::new(50.0, 150.0),
        layer_out: 0,
        layer_in: 1,
        ..Door::default()
    }];

    let ctx = AiContext {
        position: owner_position,
        ..AiContext::default()
    };
    let ai = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("owner has Enemy AI");
    ai.base.me = owner_id.index();
    ai.base.primary_target = target_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.think_recursion_depth = 1;
    ai.base.go_near(target_position, 50, GotoFlags::RUN, &ctx);
    let first_route = std::mem::take(&mut ai.base.outbox.actor);
    assert_eq!(first_route.orders.len(), 1);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(first_route));
    ai.base
        .outbox
        .reentrant
        .reconsider_approach_completion_pending = true;
    ai.base.outbox.reentrant.owner_work.push(
        AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear {
            target: target_id.index(),
            target_position,
        },
    );

    engine.drain_ai_owner_work_for(&sim, &assets, owner_id);

    let ai = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("owner retains Enemy AI");
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.last_goto_destination.x, 50.0);
    assert_eq!(ai.base.last_goto_destination.y, 100.0);
    assert!(!ai.base.couldnt_reachpoint);

    // Reachable first approaches must consume the same typed tail without
    // entering the roof-wait branch.
    let reachable_owner =
        engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let reachable_target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let reachable_owner_position = Position {
        x: 200.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let reachable_target_position = Position {
        x: 300.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    for (id, position) in [
        (reachable_owner, reachable_owner_position),
        (reachable_target, reachable_target_position),
    ] {
        let entity = engine.get_entity_mut(id).expect("reachable actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    let reachable_ctx = AiContext {
        position: reachable_owner_position,
        ..AiContext::default()
    };
    let ai = engine
        .get_entity_mut(reachable_owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("reachable owner has Enemy AI");
    ai.base.me = reachable_owner.index();
    ai.base.primary_target = reachable_target.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.think_recursion_depth = 1;
    ai.base
        .go_near(reachable_target_position, 5, GotoFlags::RUN, &reachable_ctx);
    let reachable_route = std::mem::take(&mut ai.base.outbox.actor);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(reachable_route));
    ai.base
        .outbox
        .reentrant
        .reconsider_approach_completion_pending = true;
    ai.base.outbox.reentrant.owner_work.push(
        AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear {
            target: reachable_target.index(),
            target_position: reachable_target_position,
        },
    );

    engine.drain_ai_owner_work_for(&sim, &assets, reachable_owner);

    let ai = engine
        .get_entity(reachable_owner)
        .and_then(Entity::enemy_ai)
        .expect("reachable owner retains Enemy AI");
    assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
    assert_eq!(ai.base.last_goto_destination, reachable_target_position);
    assert!(!ai.base.couldnt_reachpoint);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .reconsider_approach_completion_pending
    );
}

#[test]
fn battle_observe_route_settles_before_source_ordered_tail() {
    use crate::ai::{
        AiContext, AiOwnerWork, AiState, Decision, GotoFlags, LogLineType, Position, Substate,
    };
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, GateType};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "battle_observe_owner_boundary_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed test jump"),
    );

    // Save052's shape: the first GoNear crosses sectors and cannot construct
    // a route, while the target's jump gate provides Original's avenger-on-
    // roof recovery point.
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let owner_position = Position {
        x: 10.0,
        y: 10.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let target_position = Position {
        x: 100.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(2),
        level: 1,
    };
    for (id, position) in [(owner_id, owner_position), (target_id, target_position)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("battle-observe actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    engine
        .get_entity_mut(target_id)
        .and_then(Entity::pc_data_mut)
        .expect("battle-observe target is a PC")
        .has_jump = true;
    engine.script_domains.interactables.doors = vec![Door {
        gate_type: GateType::Jump,
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        point_out: MapPoint::new(50.0, 100.0),
        point_in: MapPoint::new(50.0, 150.0),
        layer_out: 0,
        layer_in: 1,
        ..Door::default()
    }];

    let ctx = AiContext {
        position: owner_position,
        ..AiContext::default()
    };
    let ai = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("battle-observe owner has Enemy AI");
    ai.base.me = owner_id.index();
    ai.base.primary_target = target_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.think_recursion_depth = 1;
    ai.base.outbox.actor.set_focus(target_id.index());
    ai.base
        .go_near(target_position, 50, GotoFlags::empty(), &ctx);
    let first_route = std::mem::take(&mut ai.base.outbox.actor);
    assert_eq!(first_route.focus, Some(target_id.index()));
    assert_eq!(first_route.orders.len(), 1);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(first_route));
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: target_id.index(),
            target_position,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner_id);

    let ai = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("battle-observe owner retains Enemy AI");
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.last_goto_destination.x, 50.0);
    assert_eq!(ai.base.last_goto_destination.y, 100.0);
    assert_eq!(ai.base.seek_position, target_position);
    assert!(!ai.base.couldnt_reachpoint);
    assert!(!ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert_eq!(
        ai.base
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == LogLineType::BattleDecision
                    && line.info == Decision::Observe as u16
            })
            .count(),
        0,
        "Original roof fallback returns before the Observe decision log"
    );

    // A reachable route consumes the same typed tail, enters Approach, and
    // publishes the Observe decision exactly once.
    let reachable_owner =
        engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let reachable_target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let reachable_owner_position = Position {
        x: 200.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let reachable_target_position = Position {
        x: 300.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    for (id, position) in [
        (reachable_owner, reachable_owner_position),
        (reachable_target, reachable_target_position),
    ] {
        let entity = engine.get_entity_mut(id).expect("reachable actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    let reachable_ctx = AiContext {
        position: reachable_owner_position,
        ..AiContext::default()
    };
    let ai = engine
        .get_entity_mut(reachable_owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("reachable owner has Enemy AI");
    ai.base.me = reachable_owner.index();
    ai.base.primary_target = reachable_target.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.think_recursion_depth = 1;
    ai.base.outbox.actor.set_focus(reachable_target.index());
    ai.base.go_near(
        reachable_target_position,
        5,
        GotoFlags::empty(),
        &reachable_ctx,
    );
    let reachable_route = std::mem::take(&mut ai.base.outbox.actor);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(reachable_route));
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: reachable_target.index(),
            target_position: reachable_target_position,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, reachable_owner);

    let ai = engine
        .get_entity(reachable_owner)
        .and_then(Entity::enemy_ai)
        .expect("reachable owner retains Enemy AI");
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachToObserve
    );
    assert_eq!(ai.base.last_goto_destination, reachable_target_position);
    assert!(!ai.base.couldnt_reachpoint);
    assert!(!ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert_eq!(
        ai.base
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == LogLineType::BattleDecision
                    && line.info == Decision::Observe as u16
            })
            .count(),
        1
    );
}

#[test]
#[should_panic(expected = "battle-observe continuation owner 0 has stale target 999")]
fn battle_observe_continuation_fails_loud_for_stale_target() {
    use crate::ai::{AiOwnerWork, AiState, Position, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("battle-observe owner has Enemy AI");
    ai.base.me = owner.index();
    ai.base.primary_target = 999;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: 999,
            target_position: Position::default(),
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);
}

#[test]
#[should_panic(expected = "battle-observe roof recovery requires an installed mission script")]
fn battle_observe_roof_fallback_fails_loud_without_mission() {
    use crate::ai::{AiOwnerWork, AiState, Position, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("battle-observe owner has Enemy AI");
    ai.base.me = owner.index();
    ai.base.primary_target = target.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.couldnt_reachpoint = true;
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: target.index(),
            target_position: Position::default(),
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);
}

#[test]
fn fighter_snapshot_uses_committed_gate_side_for_door_passing_actor() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let self_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));

    for (id, x) in [(self_id, 0.0), (target_id, 20.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
        soldier.element.set_position(WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
    }

    let Entity::Soldier(target) = engine
        .get_entity_mut(target_id)
        .expect("door-passing target exists")
    else {
        panic!("door-passing target changed kind")
    };
    assert!(target.actor.active_door_pass.is_none());
    let exact_target_world = WorldPoint3D::new(20.123_457, 9.876_543, 7.654_321);
    target.element.set_position_map(MapPoint::from_world_xyz(
        exact_target_world.x,
        exact_target_world.y,
        exact_target_world.z,
    ));
    target.element.set_position(exact_target_world);

    engine.script_domains.interactables.doors = vec![Door {
        door_type: DoorType::Default,
        sector_out: SectorNumber::new(7),
        sector_in: SectorNumber::new(8),
        layer_out: 3,
        layer_in: 4,
        point_out: MapPoint::new(120.0, 5.0),
        point_in: MapPoint::new(100.0, 5.0),
        ..Door::default()
    }];
    let mut pass_door = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(target_id),
        OrderType::WalkingWithSword,
    );
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass_door.data
    else {
        panic!("PassDoor test element changed kind")
    };
    *gate_id = Some(DoorIndex(0));
    *direction = 0;
    let sequence_id = engine.orders.sequence_manager.launch_element(pass_door);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    let (optical_ai_position, optical_world) =
        engine.enemy_optical_geometry_at_owner_for_test(&assets, self_id, &positions, target_id);
    assert_eq!(optical_ai_position.x, 120.0);
    assert_eq!(optical_ai_position.y, 5.0);
    assert_eq!(optical_ai_position.level, 3);
    assert_eq!(optical_world.x.to_bits(), exact_target_world.x.to_bits());
    assert_eq!(optical_world.y.to_bits(), exact_target_world.y.to_bits());
    assert_eq!(optical_world.z.to_bits(), exact_target_world.z.to_bits());

    let fighters = engine.build_nearby_fighters_for(
        self_id,
        &assets,
        &crate::sight_obstacle::SharedSightObstacles::default(),
    );
    let target = fighters
        .iter()
        .find(|fighter| fighter.handle == target_id.index())
        .expect("door-passing target remains inside the fighter radius");
    assert_eq!(target.position.x, 120.0);
    assert_eq!(target.position.y, 5.0);
    assert_eq!(
        target.position.sector,
        crate::position_interface::SectorHandle::new(7)
    );
    assert_eq!(target.position.level, 3);
}

#[test]
fn reconsider_observation_uses_raw_positions_without_changing_shared_door_snapshots() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    // Human handle zero is NULL in Original AI lists.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_near_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_far_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    for (id, x) in [(owner_id, 0.0), (raw_near_id, 20.0), (raw_far_id, 600.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
        let world = WorldPoint3D::new(x, 0.0, 0.0);
        soldier.element.set_position(world);
        soldier
            .element
            .set_position_map(MapPoint::from_world_xyz(world.x, world.y, world.z));
    }
    let frame = engine.control.frame_counter;
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("observation owner has enemy AI");
    owner.set_state(AiState::Attacking, Substate::AttackingObserve);
    owner.base.launch_timer(0, frame);
    // The engine clears the running latch when the due timer is emitted; the
    // launch substate remains as the stale-event guard consumed by Think.
    owner.base.timer_is_running = false;

    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Default,
            sector_out: SectorNumber::new(7),
            sector_in: SectorNumber::new(8),
            point_out: MapPoint::new(600.0, 0.0),
            point_in: MapPoint::new(580.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Default,
            sector_out: SectorNumber::new(9),
            sector_in: SectorNumber::new(10),
            point_out: MapPoint::new(20.0, 0.0),
            point_in: MapPoint::new(40.0, 0.0),
            ..Door::default()
        },
    ];
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "reconsider_observation_pass_door_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed test doors"),
    );
    for (id, door_index) in [(raw_near_id, 0), (raw_far_id, 1)] {
        let mut pass_door = SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(id),
            OrderType::WalkingWithSword,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass_door.data
        else {
            panic!("PassDoor test element changed kind")
        };
        *gate_id = Some(DoorIndex(door_index));
        *direction = 0;
        let sequence_id = engine.orders.sequence_manager.launch_element(pass_door);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let scratch = engine.build_sim_scratch(&sim, &assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine
            .get_entity(owner_id)
            .expect("observation owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.hiking_paths,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, owner_id, &scratch, &assets);

    assert!(
        !tick
            .nearby_fighters
            .iter()
            .any(|fighter| fighter.handle == raw_near_id.index()),
        "generic nearby scan must reject the raw-near fighter at its door-resolved far side"
    );
    assert!(
        tick.nearby_fighters
            .iter()
            .any(|fighter| fighter.handle == raw_far_id.index()),
        "generic nearby scan must retain the raw-far fighter at its door-resolved near side"
    );
    let observation_position = |handle| {
        tick.reconsider_swordfight_observation_fighters
            .iter()
            .find(|fighter| fighter.handle == handle)
            .expect("fighter exists in complete observation registry")
            .raw_world_position
    };
    assert_eq!(observation_position(raw_near_id.index()).x, 20.0);
    assert_eq!(observation_position(raw_far_id.index()).x, 600.0);

    engine.dispatch_think_with_drain(
        &sim,
        owner_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );

    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("observation owner retains enemy AI");
    assert_eq!(
        owner.base.list_us,
        vec![owner_id.index(), raw_near_id.index()]
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
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
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
    *gate_id = Some(DoorIndex(0));
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
    let scratch = engine.build_sim_scratch(&sim, &assets);
    let tick = engine.build_npc_tick_data(&sim, owner_id, &scratch, &assets);
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
    let scratch = engine.build_sim_scratch(&sim, &assets);
    let tick = engine.build_npc_tick_data(&sim, owner_id, &scratch, &assets);
    assert_eq!(tick.visible_seeking_friends, 1);
    assert!(tick.friend_seek_clears_help_flag);
}

#[test]
fn filtered_think_refreshes_live_friend_primary_target_for_battle_decisions() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::MapPoint;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let new_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    for (id, position) in [
        (owner_id, MapPoint::new(100.0, 100.0)),
        (friend_id, MapPoint::new(110.0, 100.0)),
        (old_target_id, MapPoint::new(120.0, 100.0)),
        (new_target_id, MapPoint::new(130.0, 100.0)),
    ] {
        let entity = engine.get_entity_mut(id).expect("test combatant exists");
        entity.element_data_mut().active = true;
        entity.element_data_mut().set_position_map(position);
        if let Some(npc) = entity.npc_data_mut() {
            npc.life_points = 100;
        }
    }
    for (id, substate) in [
        (owner_id, Substate::AttackingReactiontime),
        (friend_id, Substate::AttackingSwordfight),
    ] {
        let enemy = engine
            .get_entity_mut(id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test soldier has Enemy AI");
        enemy.base.me = id.index();
        enemy.set_state(AiState::Attacking, substate);
        enemy.base.primary_target = old_target_id.index();
    }
    engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("owner has Enemy AI")
        .list_them = vec![old_target_id.index()];
    let frame = engine.control.frame_counter;
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("owner has Enemy AI");
    owner.base.launch_timer(0, frame);
    owner.base.timer_is_running = false;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let scratch = engine.build_sim_scratch(&sim, &assets);
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
        &assets.hiking_paths,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, owner_id, &scratch, &assets);
    let stale_friend = tick
        .camp_soldiers
        .iter()
        .find(|friend| friend.handle == friend_id.index())
        .expect("stale tick includes the admitted friend");
    assert_eq!(stale_friend.primary_target, old_target_id.index());
    let captured_position = stale_friend.position;

    let friend = engine
        .get_entity_mut(friend_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("friend has Enemy AI");
    friend.base.primary_target = new_target_id.index();
    // Geometry remains owner-boundary data even when the live entity moves
    // after the tick snapshot was constructed.
    engine
        .get_entity_mut(friend_id)
        .expect("friend exists")
        .element_data_mut()
        .set_position_map(MapPoint::new(900.0, 900.0));

    engine.dispatch_think_with_drain(
        &sim,
        owner_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );

    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("owner retains Enemy AI");
    assert!(owner.list_them.contains(&new_target_id.index()));
    assert_eq!(
        tick.camp_soldiers
            .iter()
            .find(|friend| friend.handle == friend_id.index())
            .expect("original tick remains intact")
            .position,
        captured_position,
        "live claim refresh must not replace captured geometry"
    );
}

#[test]
fn optical_ai_position_uses_carrier_boundary_but_keeps_target_world_bits() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let carrier = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::OnShoulders));

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

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    let Entity::Pc(carrier_pc) = engine.get_entity_mut(carrier).expect("carrier PC remains") else {
        panic!("carrier changed kind")
    };
    carrier_pc
        .element
        .set_position(WorldPoint3D::new(999.0, 999.0, 99.0));
    carrier_pc
        .element
        .set_position_map(MapPoint::new(999.0, 999.0));

    let (ai_position, optical_world) =
        engine.enemy_optical_geometry_at_owner_for_test(&assets, owner, &positions, target);
    assert_eq!(ai_position.x, 321.25);
    assert_eq!(ai_position.y, 640.0);
    assert_eq!(optical_world.x.to_bits(), exact_target_world.x.to_bits());
    assert_eq!(optical_world.y.to_bits(), exact_target_world.y.to_bits());
    assert_eq!(optical_world.z.to_bits(), exact_target_world.z.to_bits());
}

fn run_synchronous_charly_report(officer_state: crate::ai::AiState) -> EngineInner {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::EyeStatus;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.world.weather.ambiance = crate::engine::types::Ambiance::Night;
    // Occupy slot 0 with a non-human entity: handle 0 is the null element
    // in AI handle space, so Charly must not land there or his viewer
    // identity cannot be resolved from the entity-view snapshot.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let charly_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for (id, x) in [(charly_id, 0.0), (officer_id, 200.0)] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("test report soldier exists")
        else {
            panic!("test report entity changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 400;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = EyeStatus::LookForward;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test report soldier has enemy AI")
            .base
            .me = id.index();
    }

    {
        let charly = engine
            .get_entity_mut(charly_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test Charly has enemy AI");
        charly.base.antagonist = officer_id.index();
        charly.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
        charly.base.launch_timer(0, 100);
        charly.base.timer_is_running = false;
    }
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test officer has enemy AI");
        let officer_substate = match officer_state {
            AiState::Default => Substate::DefaultOnPost,
            AiState::Attacking => Substate::AttackingSwordfight,
            other => panic!("unsupported Charly-report officer state: {other:?}"),
        };
        officer.set_state(officer_state, officer_substate);
    }

    let scratch = engine.build_sim_scratch(sim, &assets);
    let ctx = {
        let entity = engine
            .get_entity(charly_id)
            .expect("test Charly exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.hiking_paths,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    assert!(ctx.is_night_or_fog);
    let tick = engine.build_npc_tick_data(sim, charly_id, &scratch, &assets);
    engine.dispatch_think_with_drain(
        sim,
        charly_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );
    engine
}

#[test]
fn charly_report_uses_synchronous_officer_acceptance_and_refusal() {
    use crate::ai::{AiState, Substate};

    let accepted = run_synchronous_charly_report(AiState::Default);
    let charly = accepted
        .world
        .entities
        .soldiers()
        .next()
        .expect("accepted Charly exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("accepted Charly has enemy AI");
    assert_eq!(
        charly.base.current_substate,
        Substate::SeekingCharlyGoToOfficerSeen
    );
    assert_eq!(charly.base.when_does_timer_ring, 110);

    let refused = run_synchronous_charly_report(AiState::Attacking);
    let charly = refused
        .world
        .entities
        .soldiers()
        .next()
        .expect("refused Charly exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("refused Charly has enemy AI");
    assert_eq!(charly.base.current_state, AiState::Default);
    assert_ne!(
        charly.base.current_substate,
        Substate::SeekingCharlyGoToOfficerSeen
    );
}

fn run_synchronous_civilian_alert(
    soldier_state: crate::ai::AiState,
    trigger: crate::ai::StimulusType,
    direct_owner_self_stimulus: bool,
) -> EngineInner {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::AiBrain;

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("test civilian exists")
    else {
        panic!("test civilian changed kind")
    };
    civilian.element.active = true;
    civilian.element.set_position_map(MapPoint::new(0.0, 0.0));
    civilian.civilian.cached_camp = crate::element::Camp::Royalists;
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        civilian_id.index(),
    )));
    let friendly = civilian
        .npc
        .ai_brain
        .friendly_mut()
        .expect("test civilian has FriendlyAi");
    friendly.base.owner_entity_id = Some(civilian_id);
    friendly.base.antagonist = soldier_id.index();
    friendly.base.my_reconnaissance_report.update(
        crate::ai::ReportType::Enemy,
        crate::ai::Position {
            x: 300.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
    );
    friendly.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
    civilian.npc.detectable_lists[crate::element::DetectableType::Friend as usize].push(
        crate::element::Detectable {
            element: Some(soldier_id),
            detectable_type: crate::element::DetectableType::Friend,
            ..Default::default()
        },
    );

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("test soldier exists")
    else {
        panic!("test soldier changed kind")
    };
    soldier.element.active = true;
    soldier.element.set_position_map(MapPoint::new(20.0, 0.0));
    let enemy = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has EnemyAi");
    enemy.base.me = soldier_id.index();
    enemy.base.owner_entity_id = Some(soldier_id);
    enemy.set_state(
        soldier_state,
        if soldier_state == AiState::Default {
            Substate::DefaultOnPost
        } else {
            Substate::AttackingSwordfight
        },
    );

    complete_test_runtime_fixture(&mut engine, &mut assets);

    let scratch = engine.build_sim_scratch(sim, &assets);
    let ctx = {
        let entity = engine
            .get_entity(civilian_id)
            .expect("test civilian exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.hiking_paths,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    let tick = engine.build_npc_tick_data(sim, civilian_id, &scratch, &assets);
    if direct_owner_self_stimulus {
        engine
            .get_entity_mut(civilian_id)
            .and_then(Entity::friendly_ai_mut)
            .expect("direct-owner civilian has FriendlyAi")
            .base
            .outbox
            .reentrant
            .self_stimuli
            .push(trigger.into());
        engine.drain_direct_ai_owner_boundary(sim, civilian_id, &assets);
    } else {
        let stimulus = if trigger == StimulusType::EventSeesSoldier {
            Stimulus::with_human(trigger, soldier_id.index())
        } else {
            Stimulus::new(trigger)
        };
        engine.dispatch_think_with_drain(sim, civilian_id, &stimulus, &ctx, &tick, &assets);
    }
    engine
}

#[test]
fn civilian_alert_closes_recipient_and_result_continuation_synchronously() {
    use crate::ai::{AiState, StimulusType, Substate};

    let mut accepted =
        run_synchronous_civilian_alert(AiState::Default, StimulusType::EventReachPoint, false);
    let civilian = accepted
        .world
        .entities
        .civilians()
        .next()
        .expect("accepted civilian exists")
        .1
        .npc
        .ai_brain
        .friendly()
        .expect("accepted civilian has FriendlyAi");
    assert!(
        accepted
            .world
            .entities
            .civilians()
            .next()
            .expect("accepted civilian exists")
            .1
            .npc
            .detectable_lists[crate::element::DetectableType::Friend as usize]
            .is_empty(),
        "reached-soldier acceptance deletes friend detectables"
    );
    assert_eq!(
        civilian.base.current_substate,
        Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
        "recipient CALL_ALERT and recursive EVENT_REACHPOINT must settle before dispatch returns"
    );
    assert_eq!(civilian.base.when_does_timer_ring, 110);
    let soldier = accepted
        .world
        .entities
        .soldiers()
        .next()
        .expect("accepting soldier exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("accepting soldier has EnemyAi");
    assert_eq!(
        soldier.base.current_substate,
        Substate::SeekingWaitForAlertingCivilian
    );

    let civilian_id = accepted
        .world
        .entities
        .civilians()
        .next()
        .expect("reporting civilian exists")
        .0;
    accepted.control.frame_counter = 110;
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut accepted, &mut assets);
    let sim_context = crate::sim_rng::test_context();
    let scratch = accepted.build_sim_scratch(&sim_context, &assets);
    let ctx = {
        let entity = accepted
            .get_entity(EntityId::Civilian(civilian_id))
            .expect("reporting civilian exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            accepted.control.frame_counter,
            None,
            accepted.world.weather.is_forest_level,
            accepted.world.weather.ambiance,
            accepted.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &accepted.world.fast_grid,
            &assets.hiking_paths,
            &accepted.ai.global.all_soldier_handles,
            accepted.control.sim_config.difficulty,
        )
    };
    let civilian_entity_id = EntityId::Civilian(civilian_id);
    let tick = accepted.build_npc_tick_data(&sim_context, civilian_entity_id, &scratch, &assets);
    accepted.dispatch_think_with_drain(
        &sim_context,
        civilian_entity_id,
        &crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );
    let soldier = accepted
        .world
        .entities
        .soldiers()
        .next()
        .expect("reported-to soldier exists")
        .1
        .npc
        .ai_brain
        .enemy()
        .expect("reported-to soldier has EnemyAi");
    assert_eq!(
        soldier.base.current_substate,
        Substate::SeekingGetAlertingReportFromCivilian,
        "CALL_REPORT recipient transition must settle before the civilian timer dispatch returns"
    );
    assert_eq!(
        soldier.base.my_reconnaissance_report.report_type,
        crate::ai::ReportType::Enemy
    );

    let refused =
        run_synchronous_civilian_alert(AiState::Attacking, StimulusType::EventReachPoint, false);
    let civilian = refused
        .world
        .entities
        .civilians()
        .next()
        .expect("refused civilian exists")
        .1
        .npc
        .ai_brain
        .friendly()
        .expect("refused civilian has FriendlyAi");
    assert_ne!(civilian.base.current_state, AiState::Seeking);
    assert_ne!(
        civilian.base.current_substate,
        Substate::SeekingCivilianRunningToSoldierSeen
    );
}

#[test]
fn review_civilian_sees_soldier_deletes_friends_before_acceptance_or_refusal() {
    use crate::ai::{AiState, StimulusType};
    use crate::element::DetectableType;

    for state in [AiState::Default, AiState::Attacking] {
        let engine = run_synchronous_civilian_alert(state, StimulusType::EventSeesSoldier, false);
        let civilian = engine
            .world
            .entities
            .civilians()
            .next()
            .expect("civilian exists")
            .1;
        assert!(
            civilian.npc.detectable_lists[DetectableType::Friend as usize].is_empty(),
            "EVENT_SEES_SOLDIER must delete friends before CALL_ALERT for {state:?} recipient"
        );
    }
}

#[test]
fn review_direct_owner_self_stimulus_closes_nested_alert_request() {
    use crate::ai::{AiState, StimulusType, Substate};

    let engine =
        run_synchronous_civilian_alert(AiState::Default, StimulusType::EventReachPoint, true);
    let civilian = engine
        .world
        .entities
        .civilians()
        .next()
        .expect("civilian exists")
        .1
        .npc
        .ai_brain
        .friendly()
        .expect("civilian has FriendlyAi");
    assert_eq!(
        civilian.base.current_substate,
        Substate::SeekingCivilianGiveAlertingReportToSoldierStart
    );
    assert!(
        !civilian.base.has_pending_synchronous_cross_npc_actions(),
        "nested RequestAlert must not escape the direct-owner fixed point"
    );
}

#[test]
fn review_officer_call_hey_refusal_returns_to_duty_synchronously() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    // AI human handles use zero as NULL, so keep production NPCs off slot 0.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for (id, rank, state, substate) in [
        (
            officer_id,
            ProfileRank::Officer,
            AiState::Seeking,
            Substate::SeekingOfficerCallSoldier,
        ),
        (
            soldier_id,
            ProfileRank::Soldier,
            AiState::Attacking,
            Substate::AttackingSwordfight,
        ),
    ] {
        let enemy = engine
            .get_entity_mut(id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test soldier has EnemyAi");
        enemy.base.me = id.index();
        enemy.soldier_profile_rank = rank;
        enemy.set_state(state, substate);
    }
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("officer has EnemyAi")
        .base
        .antagonist = soldier_id.index();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let scratch = engine.build_sim_scratch(&sim, &assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(officer_id).expect("officer exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.hiking_paths,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, officer_id, &scratch, &assets);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::new(StimulusType::EventDone),
        &ctx,
        &tick,
        &assets,
    );

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("officer retains EnemyAi");
    assert_ne!(
        officer.base.current_substate,
        Substate::SeekingOfficerWaitForSoldier
    );
    assert_eq!(officer.base.current_state, AiState::Default);
}

#[test]
#[should_panic(expected = "EVENT_SEES_SOLDIER target 1 must have soldier rank")]
fn review_officer_sees_soldier_rejects_non_soldier_rank_target() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for id in [officer_id, target_id] {
        let enemy = engine
            .get_entity_mut(id)
            .and_then(Entity::enemy_ai_mut)
            .expect("officer test entity has EnemyAi");
        enemy.base.me = id.index();
        enemy.soldier_profile_rank = ProfileRank::Officer;
        enemy.set_state(AiState::Default, Substate::DefaultOnPost);
    }
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let scratch = engine.build_sim_scratch(&sim, &assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(officer_id).expect("officer exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.hiking_paths,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, officer_id, &scratch, &assets);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::with_human(StimulusType::EventSeesSoldier, target_id.index()),
        &ctx,
        &tick,
        &assets,
    );
}

#[test]
fn review_soldier_alert_uses_live_caller_after_recipient_callback() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::profiles::ProfileRank;

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let reporter_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let callback_officer_id =
        engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for (id, x, rank) in [
        (reporter_id, 0.0, ProfileRank::Soldier),
        (officer_id, 40.0, ProfileRank::Officer),
        (callback_officer_id, 80.0, ProfileRank::Officer),
    ] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("alert soldier exists")
        else {
            panic!("alert soldier changed kind")
        };
        soldier.element.active = true;
        soldier.element.sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
        );
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("alert soldier has EnemyAi");
        ai.base.me = id.index();
        ai.soldier_profile_rank = rank;
        ai.set_state(AiState::Default, Substate::DefaultOnPost);
    }

    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("accepting officer has EnemyAi")
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(crate::ai::CrossNpcAction::SendStimulus {
            target: reporter_id.index(),
            stimulus_type: StimulusType::CallAlert,
            info: crate::ai::StimulusInfo::Human(callback_officer_id.index()),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    let scratch = engine.build_sim_scratch(sim, &assets);
    let ctx = {
        let entity = engine
            .get_entity(reporter_id)
            .expect("reporter exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.hiking_paths,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    let tick = engine.build_npc_tick_data(sim, reporter_id, &scratch, &assets);
    engine.dispatch_think_with_drain(
        sim,
        reporter_id,
        &Stimulus::with_human(StimulusType::EventSeesSoldier, officer_id.index()),
        &ctx,
        &tick,
        &assets,
    );

    let reporter = engine
        .get_entity(reporter_id)
        .and_then(Entity::enemy_ai)
        .expect("reporter retains EnemyAi");
    assert_eq!(
        reporter.base.current_substate,
        Substate::SeekingRunningToOfficerSeen
    );
    assert_eq!(
        reporter.base.antagonist,
        callback_officer_id.index(),
        "recipient callback must be allowed to mutate the suspended caller"
    );
    assert_eq!(
        reporter.base.last_goto_destination.x, 80.0,
        "outer continuation must resume from the caller's live antagonist"
    );
    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("officer retains EnemyAi");
    assert_eq!(
        officer.base.current_substate,
        Substate::SeekingOfficerWaitForAlertingSoldier,
        "soldier caller must not be routed through the civilian CALL_ALERT arm"
    );
}

fn setup_review2_officer_and_soldier() -> (EngineInner, EntityId, EntityId, LevelAssets) {
    use crate::ai::{AiState, Substate};
    use crate::profiles::ProfileRank;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    // AI human handles use zero as NULL, so keep production NPCs off slot 0.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Target,
            ..Default::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let officer_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for (id, rank, x) in [
        (officer_id, ProfileRank::Officer, 0.0),
        (soldier_id, ProfileRank::Soldier, 40.0),
    ] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("review2 soldier exists")
        else {
            panic!("review2 entity changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.element.index_in_elements_list = id.index() as u16;
        soldier.npc.life_points = 100;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("review2 soldier has EnemyAi");
        ai.base.me = id.index();
        ai.soldier_profile_rank = rank;
        ai.set_state(AiState::Default, Substate::DefaultOnPost);
    }
    complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, officer_id, soldier_id, assets)
}

fn review2_context_and_tick(
    engine: &EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    id: EntityId,
) -> (crate::ai::AiContext, crate::ai::AiPerTickData) {
    let scratch = engine.build_sim_scratch(sim, assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(id).expect("review2 context owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.hiking_paths,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(sim, id, &scratch, assets);
    (ctx, tick)
}

#[test]
fn officer_call_rejection_closes_return_to_duty_actor_fixed_point() {
    use crate::ai::{AiState, CrossNpcAction, Substate, ThinkResultContinuation};
    use crate::element::{Command, Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    engine.drain_direct_ai_owner_boundary(&sim, soldier_id, &assets);

    let sector = crate::position_interface::SectorHandle::new(1);
    let officer = engine
        .get_entity_mut(officer_id)
        .expect("call-rejection officer exists");
    officer
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    officer.element_data_mut().set_sector(sector);
    let officer_npc = officer.npc_data_mut().expect("officer remains NPC");
    officer_npc.detectable_lists[DetectableType::Beggar as usize].push(Detectable {
        element: Some(soldier_id),
        detectable_type: DetectableType::Beggar,
        ..Default::default()
    });
    let officer_ai = officer
        .enemy_ai_mut()
        .expect("call-rejection officer has EnemyAi");
    officer_ai.base.antagonist = soldier_id.index();
    officer_ai.base.initial_position = crate::ai::Position {
        x: 200.0,
        y: 100.0,
        sector,
        level: 0,
    };
    officer_ai.attentive = true;
    officer_ai.will_be_attentive = true;
    officer_ai.base.current_state = AiState::Seeking;
    officer_ai.base.current_substate = Substate::SeekingOfficerCallSoldier;
    officer_ai
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestThinkResult {
            target: soldier_id.index(),
            caller: officer_id.index(),
            stimulus_type: crate::ai::StimulusType::CallHey,
            info: crate::ai::StimulusInfo::Human(officer_id.index()),
            continuation: ThinkResultContinuation::OfficerCalledSoldier,
        });
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("call rejector has EnemyAi")
        .set_state(AiState::Attacking, Substate::AttackingSwordfight);

    engine.process_synchronous_think_results_for(&sim, officer_id, &assets, true);

    let officer = engine
        .get_entity(officer_id)
        .expect("call-rejection officer survives");
    let ai = officer
        .enemy_ai()
        .expect("call-rejection officer retains EnemyAi");
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert!(
        officer
            .npc_data()
            .expect("officer remains NPC")
            .detectable_lists[DetectableType::Beggar as usize]
            .is_empty(),
        "ReturnToDuty's Beggar deletion must settle on the resumed caller stack"
    );
    assert!(!ai.base.outbox.actor.has_boundary_work());
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());

    let commands: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(officer_id))
        .map(|element| (element.command, element.state))
        .collect();
    let leave = commands
        .iter()
        .position(|(command, _)| *command == Command::LeaveAttentiveMode)
        .expect("rejected call must publish LeaveAttentiveMode");
    let movement = commands
        .iter()
        .position(|(command, _)| *command == Command::Move)
        .expect("rejected call must publish the return-to-post movement");
    assert!(
        leave < movement,
        "LeaveAttentiveMode must precede GoTo: {commands:?}"
    );
    assert_eq!(commands[leave].1, crate::sequence::SequenceState::Todo);
    assert_eq!(commands[movement].1, crate::sequence::SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(officer_id)
            .is_none(),
        "deferred owner mode must not instruct either element"
    );

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    let current = engine
        .orders
        .sequence_manager
        .current_element_for_actor(officer_id)
        .and_then(|(sequence, index)| engine.orders.sequence_manager.get_element(sequence, index))
        .expect("manager phase must select the attentive transition");
    assert_eq!(current.command, Command::LeaveAttentiveMode);
    assert_eq!(current.state, crate::sequence::SequenceState::InProgress);
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(officer_id)
                    && element.command == Command::Move
                    && element.state == crate::sequence::SequenceState::InProgress
            }),
        "return-to-post movement must remain behind LeaveAttentiveMode"
    );
}

#[test]
fn resumed_return_to_duty_translates_its_goto_on_the_owner_work_boundary() {
    use crate::ai::{AiOwnerWork, AiState, DutyFlags, Substate};
    use crate::coordinates::MapPoint;
    use crate::element::Command;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let sector = crate::position_interface::SectorHandle::new(1);
    let entity = engine
        .get_entity_mut(owner)
        .expect("return-to-duty owner exists");
    entity.element_data_mut().active = true;
    entity
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    entity.element_data_mut().set_sector(sector);
    let ai = entity
        .enemy_ai_mut()
        .expect("return-to-duty owner has Enemy AI");
    ai.base.me = owner.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGetInstructedByOfficer;
    ai.base.initial_position = crate::ai::Position {
        x: 300.0,
        y: 100.0,
        sector,
        level: 0,
    };
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
            flags: DutyFlags::empty(),
            defer_clear_patrol_close_post: false,
            owner_boundary_positions: vec![(
                owner.index(),
                crate::ai::Position {
                    x: 100.0,
                    y: 100.0,
                    sector,
                    level: 0,
                },
            )],
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("return-to-duty owner retains Enemy AI");
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert!(
        !ai.base.outbox.actor.has_boundary_work(),
        "ReturnToDutyCommonStuff's GoTo must be translated before the resumed owner work returns"
    );
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| element.owner == Some(owner) && element.command == Command::Move),
        "the return-to-post GoTo must already exist in the sequence manager"
    );
}

#[test]
fn officer_call_acceptance_keeps_wait_state_timer_and_beggar() {
    use crate::ai::{AiState, CrossNpcAction, Substate, ThinkResultContinuation};
    use crate::element::{Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    engine.drain_direct_ai_owner_boundary(&sim, soldier_id, &assets);
    let officer = engine
        .get_entity_mut(officer_id)
        .expect("call-acceptance officer exists");
    officer
        .npc_data_mut()
        .expect("officer remains NPC")
        .detectable_lists[DetectableType::Beggar as usize]
        .push(Detectable {
            element: Some(soldier_id),
            detectable_type: DetectableType::Beggar,
            ..Default::default()
        });
    let officer_ai = officer
        .enemy_ai_mut()
        .expect("call-acceptance officer has EnemyAi");
    officer_ai.base.antagonist = soldier_id.index();
    officer_ai.attentive = true;
    officer_ai.will_be_attentive = true;
    officer_ai.base.current_state = AiState::Seeking;
    officer_ai.base.current_substate = Substate::SeekingOfficerCallSoldier;
    officer_ai
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestThinkResult {
            target: soldier_id.index(),
            caller: officer_id.index(),
            stimulus_type: crate::ai::StimulusType::CallHey,
            info: crate::ai::StimulusInfo::Human(officer_id.index()),
            continuation: ThinkResultContinuation::OfficerCalledSoldier,
        });

    engine.process_synchronous_think_results_for(&sim, officer_id, &assets, true);

    let officer = engine
        .get_entity(officer_id)
        .expect("call-acceptance officer survives");
    let ai = officer
        .enemy_ai()
        .expect("call-acceptance officer retains EnemyAi");
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingOfficerWaitForSoldier
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(
        ai.base.when_does_timer_ring,
        engine.control.frame_counter + 20
    );
    assert_eq!(ai.base.antagonist, soldier_id.index());
    assert_eq!(
        officer
            .npc_data()
            .expect("officer remains NPC")
            .detectable_lists[DetectableType::Beggar as usize]
            .first()
            .map(|detectable| (detectable.element, detectable.detectable_type)),
        Some((Some(soldier_id), DetectableType::Beggar)),
        "accepted CALL_HEY must not enter ReturnToDuty"
    );
    assert_eq!(
        officer
            .npc_data()
            .expect("officer remains NPC")
            .detectable_lists[DetectableType::Beggar as usize]
            .len(),
        1
    );
    assert!(!ai.base.outbox.actor.has_boundary_work());
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(officer_id)
                    && element.command == crate::element::Command::LeaveAttentiveMode
            }),
        "accepted CALL_HEY must not publish a return-to-duty transition"
    );
    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("accepted soldier retains EnemyAi");
    assert_eq!(
        soldier.base.current_substate,
        Substate::SeekingSoldierCalledByOfficer
    );
    assert_eq!(soldier.base.antagonist, officer_id.index());
}

#[test]
fn nested_reentrant_turn_remains_deferred_until_manager() {
    use crate::ai::{AiState, CrossNpcAction, StimulusInfo, StimulusType, Substate};
    use crate::element::Command;
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let (mut engine, source_id, target_id, assets) = setup_review2_officer_and_soldier();
    engine.drain_direct_ai_owner_boundary(&sim, source_id, &assets);
    engine.drain_direct_ai_owner_boundary(&sim, target_id, &assets);

    engine
        .get_entity_mut(source_id)
        .expect("nested-turn source exists")
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 0.0));
    let target = engine
        .get_entity_mut(target_id)
        .expect("nested-turn target exists");
    target
        .element_data_mut()
        .set_position_map(MapPoint::new(40.0, 0.0));
    let target_ai = target
        .enemy_ai_mut()
        .expect("nested-turn target has EnemyAi");
    target_ai.base.current_state = AiState::Seeking;
    target_ai.base.current_substate = Substate::SeekingOfficerWaitForCharly;
    target_ai.base.antagonist = source_id.index();

    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .expect("nested-turn source has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::SendStimulus {
            target: target_id.index(),
            stimulus_type: StimulusType::CallCoordinate,
            info: StimulusInfo::Human(source_id.index()),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    engine.drain_direct_ai_owner_boundary_mode(&sim, source_id, &assets, true, true);

    let turns: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(target_id) && element.command == Command::Turn)
        .map(|element| element.state)
        .collect();
    assert_eq!(turns, [SequenceState::Todo]);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(target_id)
            .is_none(),
        "nested Turn must remain uninstructed until the manager hourglass"
    );
    assert_eq!(
        engine
            .get_entity(target_id)
            .and_then(Entity::enemy_ai)
            .expect("nested-turn target retains EnemyAi")
            .base
            .current_substate,
        Substate::SeekingOfficerLectureCharly
    );
}

#[test]
fn blipped_report_speech_callback_precedes_give_report_state_and_timer() {
    use crate::ai::{AiState, LogLineType, Stimulus, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("report officer has EnemyAi");
        officer.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerWaitForInstructedGroup,
        );
    }
    {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("reporting soldier exists")
        else {
            panic!("reporting soldier changed kind")
        };
        soldier.element.blipped = true;
        let reporter = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("reporting soldier has EnemyAi");
        reporter.base.antagonist = officer_id.index();
        reporter.set_state(AiState::Seeking, Substate::SeekingSoldierReturnToOfficer);
    }

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, soldier_id);
    engine.dispatch_think_with_drain(
        &sim,
        soldier_id,
        &Stimulus::new(StimulusType::EventReachPoint),
        &ctx,
        &tick,
        &assets,
    );

    let reporter = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("reporting soldier retains EnemyAi");
    assert_eq!(
        reporter.base.current_substate,
        Substate::SeekingSoldierGiveReportToOfficer
    );
    assert!(reporter.base.timer_is_running);
    assert_eq!(
        reporter.base.when_does_timer_ring, 200,
        "the rejected MYTALK callback runs in the return-to-officer substate; the later 100-frame timer must win"
    );
    assert!(
        reporter
            .base
            .ai_log
            .iter()
            .any(|line| { line.line_type == LogLineType::SpeakImpossible && line.info == 0 })
    );
    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("report officer retains EnemyAi");
    assert!(officer.base.ai_log.iter().any(|line| {
        line.line_type == LogLineType::Event && line.info == StimulusType::CallReport as u16
    }));
    assert!(!officer.base.ai_log.iter().any(|line| {
        line.line_type == LogLineType::Event && line.info == StimulusType::CallYourTalk1 as u16
    }));
    assert!(reporter.base.outbox.reentrant.owner_work.is_empty());
    assert!(reporter.base.outbox.reentrant.cross_npc_actions.is_empty());
}

fn start_review_command_soldiers(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    officer_id: EntityId,
) -> (
    crate::ai_enemy::CommandSoldiersStart,
    crate::ai::AiPerTickData,
) {
    use crate::ai::Position;

    let (ctx, tick) = review2_context_and_tick(engine, sim, assets, officer_id);
    let global = engine.ai.global.clone();
    let start = engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review command caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    (start, tick)
}

#[test]
fn review2_call_instruction_uses_refusal_to_prune_group_synchronously() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("review2 officer has EnemyAi");
        officer.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerInstructGroupPointing,
        );
        officer.alerted_us = vec![soldier_id.index()];
    }
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::new(StimulusType::EventDone),
        &ctx,
        &tick,
        &assets,
    );

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 officer retains EnemyAi");
    assert!(officer.alerted_us.is_empty());
    assert_eq!(officer.base.current_state, AiState::Default);
}

#[test]
fn review2_accepted_group_instruction_closes_officer_state_callback() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("review2 officer has EnemyAi");
        officer.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerInstructGroupPointing,
        );
        officer.alerted_us = vec![soldier_id.index()];
    }
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 instructed soldier has EnemyAi")
        .set_state(
            AiState::Seeking,
            Substate::SeekingGroupGetInstructedByOfficer,
        );

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::new(StimulusType::EventDone),
        &ctx,
        &tick,
        &assets,
    );

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 officer retains EnemyAi");
    assert_eq!(
        officer.base.current_substate,
        Substate::SeekingOfficerWaitForInstructedGroup
    );
    assert!(
        officer.base.outbox.reentrant.owner_work.is_empty(),
        "the continuation's SetState callback escaped the direct Think boundary"
    );
}

#[test]
fn review2_alert_soldiers_uses_state_refusal_and_does_not_consider_report() {
    use crate::ai::{AiState, CrossNpcAction, Position, ReportType, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("review2 officer has EnemyAi");
        officer.base.my_reconnaissance_report.report_type = ReportType::Enemy;
        officer.base.my_reconnaissance_report.seek_position = Position {
            x: 10.0,
            y: 20.0,
            ..Default::default()
        };
    }
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 officer has EnemyAi")
        .alert_soldiers(
            Position {
                x: 100.0,
                ..Default::default()
            },
            0,
            &global,
            None,
            &ctx,
            &tick,
            crate::ai::AlertSoldiersFailureContinuation::None,
        );
    // The candidate snapshot admitted this soldier, but the live recipient
    // changes before the direct call and refuses it.
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 alerted soldier has EnemyAi")
        .set_state(AiState::Attacking, Substate::AttackingSwordfight);
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 officer retains AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::InstructGatherPosition {
            target: soldier_id.index(),
            position: Position {
                x: 33.0,
                ..Default::default()
            },
            direction: 4,
            call_instruction: false,
        });
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 officer retains EnemyAi");
    assert!(officer.alerted_us.is_empty());
    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 soldier retains EnemyAi");
    assert_eq!(
        soldier.base.my_reconnaissance_report.report_type,
        ReportType::Nothing,
        "a refused CALL_ALERT must not run ConsiderReport"
    );
    assert!(
        !soldier.gather_position_instructed,
        "a refused alert target must not receive its precomputed gather instruction"
    );
}

#[test]
fn review2_combat_alert_preserves_original_busy_lock_acceptance() {
    use crate::ai::{AiLockFlags, Position, StimulusType};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 combat-alert soldier has AI")
        .locks_flag_field = AiLockFlags::BUSY;
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 officer has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 100.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    assert_eq!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("review2 officer retains EnemyAi")
            .alerted_us
            .as_slice(),
        &[soldier_id.index()],
        "Original Think returns true when StartThink retains a BUSY stimulus"
    );
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::ai_controller)
            .expect("review2 combat-alert soldier retains AI")
            .stimulus_queue
            .last()
            .map(|s| s.stimulus_type),
        Some(StimulusType::CallCombatAlert)
    );
}

#[test]
fn unalert_charly_seekers_uses_full_visibility_in_original_short_circuit_order() {
    use crate::ai::{AiState, CharlySeekerTarget, StimulusType, Substate};
    use crate::coordinates::WorldPoint3D;
    use crate::element::{AiBrain, Command, Detectable, DetectableType, Posture};
    use crate::position_interface::Direction;
    use crate::sequence::SequenceState;
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

    fn add_enemy(
        engine: &mut EngineInner,
        position: WorldPoint3D,
        direction: Direction,
    ) -> EntityId {
        let mut entity = make_test_soldier(Posture::Upright);
        let Entity::Soldier(soldier) = &mut entity else {
            unreachable!();
        };
        soldier.element.active = true;
        soldier.element.set_position(position);
        soldier
            .element
            .set_direction_instantly(direction.as_u8() as i16);
        soldier.npc.life_points = 60;
        soldier.npc.view_radius = 400;
        soldier.npc.view_radius_base = 400;
        soldier.npc.view_radius_goal = 400;
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        engine.add_entity(entity)
    }

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.ai.standard_view_polygon_radius = 400;
    // Reserve legacy handle zero: HumanHandle 0 is the original null
    // sentinel and must not be used as a real visibility target.
    let sentinel = add_enemy(
        &mut engine,
        WorldPoint3D::new(-1000.0, -1000.0, 0.0),
        Direction::EAST,
    );
    engine
        .get_entity_mut(sentinel)
        .expect("sentinel fixture exists")
        .element_data_mut()
        .active = false;
    let owner = add_enemy(
        &mut engine,
        WorldPoint3D::new(200.0, 100.0, 0.0),
        Direction::EAST,
    );
    let charly = add_enemy(
        &mut engine,
        WorldPoint3D::new(200.0, 0.0, 0.0),
        Direction::EAST,
    );
    // Charly is behind an opaque wall. This candidate pins the full
    // visibility rejection path rather than being admitted by raw geometry.
    let second_arm = add_enemy(
        &mut engine,
        WorldPoint3D::new(0.0, 0.0, 0.0),
        Direction::EAST,
    );
    // Charly is directly visible from here. Original short-circuits before
    // evaluating owner, so this candidate contributes exactly one query.
    let first_arm = add_enemy(
        &mut engine,
        WorldPoint3D::new(200.0, -100.0, 0.0),
        Direction::EAST,
    );
    // The close side-on special case succeeds before ComputeViewRadius/LOS.
    let near_side = add_enemy(
        &mut engine,
        WorldPoint3D::new(200.0, 20.0, 0.0),
        Direction::EAST,
    );
    // This candidate would see Charly, but is the owner's antagonist at the
    // synchronous sweep call boundary and must therefore be skipped.
    let excluded_antagonist = add_enemy(
        &mut engine,
        WorldPoint3D::new(200.0, -150.0, 0.0),
        Direction::EAST,
    );

    for candidate in [second_arm, first_arm, near_side, excluded_antagonist] {
        let soldier = engine
            .get_entity_mut(candidate)
            .and_then(|entity| match entity {
                Entity::Soldier(soldier) => Some(soldier),
                _ => None,
            })
            .expect("Charly-seeker candidate is a soldier");
        let enemy = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("Charly-seeker candidate has EnemyAi");
        enemy.set_state(AiState::Seeking, Substate::SeekingBody);
        enemy.base.my_reconnaissance_report.charly = charly.index();
        enemy.base.checkpoint_charly = charly.index();
        enemy.base.sorrow_level = 37;
        soldier.npc.detectable_lists[DetectableType::MissedFriend as usize].push(Detectable {
            element: Some(charly),
            detectable_type: DetectableType::MissedFriend,
            ..Detectable::default()
        });
    }
    {
        let owner_ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("Unalert owner has EnemyAi");
        owner_ai.soldier_profile_rank = crate::profiles::ProfileRank::Soldier;
        owner_ai.base.antagonist = excluded_antagonist.index();
        owner_ai
            .base
            .outbox
            .actor
            .queue_unalert_near_charly_seekers(
                CharlySeekerTarget::Npc(charly.index()),
                owner_ai.base.antagonist,
            );
        // Model rejected speech running ReturnToDuty after the Original call
        // but before Rust drains the engine-side sweep.
        owner_ai.base.antagonist = 0;
    }

    let mut wall = SightObstacle::new_default(1);
    wall.obstacle_points = vec![
        ObstaclePoint {
            x: 95.0,
            y: -10.0,
            z_bottom: 0.0,
            z_top: 100.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: -10.0,
            z_bottom: 0.0,
            z_top: 100.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: 10.0,
            z_bottom: 0.0,
            z_top: 100.0,
        },
        ObstaclePoint {
            x: 95.0,
            y: 10.0,
            z_bottom: 0.0,
            z_top: 100.0,
        },
    ];
    wall.top_plane_points = [
        [95.0, -10.0, 100.0],
        [105.0, -10.0, 100.0],
        [105.0, 10.0, 100.0],
    ];
    wall.bottom_plane_points = [[95.0, -10.0, 0.0], [105.0, -10.0, 0.0], [105.0, 10.0, 0.0]];
    wall.rebuild_geometry();
    let mut assets = LevelAssets::new();
    assets.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
    engine.world.static_sight_obstacle_active = vec![true];
    complete_test_runtime_fixture(&mut engine, &mut assets);

    crate::sight_obstacle::begin_parity_visibility_capture();
    engine.drain_direct_ai_owner_boundary(&sim, owner, &assets);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(
        queries.iter().map(|query| query.result).collect::<Vec<_>>(),
        [false, true, true],
        "blocked Charly must fall through to owner; clear Charly must short-circuit owner; near-side must not query"
    );
    assert_eq!(queries[0].destination[0], 200.0);
    assert_eq!(queries[0].destination[1], 0.0);
    assert_eq!(queries[1].destination[0], 200.0);
    assert_eq!(queries[1].destination[1], 100.0);
    assert_eq!(queries[2].destination[0], 200.0);
    assert_eq!(queries[2].destination[1], 0.0);
    assert!(
        engine
            .get_entity(owner)
            .and_then(Entity::ai_controller)
            .expect("Unalert owner retains AI")
            .outbox
            .actor
            .unalert_near_charly_seekers
            .is_none(),
        "the real pending action must be consumed"
    );
    for candidate in [second_arm, first_arm, near_side] {
        let soldier = engine
            .get_entity(candidate)
            .and_then(|entity| match entity {
                Entity::Soldier(soldier) => Some(soldier),
                _ => None,
            })
            .expect("admitted candidate remains a soldier");
        let enemy = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("admitted candidate retains EnemyAi");
        assert_eq!(
            enemy.base.current_substate,
            Substate::SeekingLookingResurrectedCharly,
            "candidate {} must receive the Charly callback",
            candidate.index()
        );
        assert!(
            enemy
                .base
                .ai_log
                .iter()
                .any(|line| line.info == StimulusType::CallCharlyIsBack as u16),
            "the admitted recipient must synchronously receive CALL_CHARLY_IS_BACK"
        );
        assert_eq!(enemy.base.checkpoint_charly, 0);
        assert_eq!(enemy.base.sorrow_level, 0);
        assert!(
            soldier.npc.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
            "SetCheckpointCharly(NULL) must synchronously clear the recipient's missed-friend list"
        );
        assert!(enemy.base.outbox.actor.delete_detectables.is_empty());
    }
    assert_eq!(
        engine
            .get_entity(excluded_antagonist)
            .and_then(Entity::enemy_ai)
            .expect("excluded antagonist retains EnemyAi")
            .base
            .current_substate,
        Substate::SeekingBody,
        "the sweep must use the call-boundary antagonist after the owner's live field is cleared"
    );
    assert_eq!(
        u8::from(
            engine
                .get_entity(first_arm)
                .expect("deferred-face candidate exists")
                .position_iface()
                .get_direction()
        ),
        Direction::EAST.as_u8(),
        "the synchronous callback must register Face without instructing Turn in the actor slot"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(first_arm)
                    && element.command == Command::Turn
                    && element.state == SequenceState::Todo
            }),
        "the admitted recipient's Face must remain a deferred standalone Turn"
    );
}

#[test]
fn final_review_alert_all_refused_resumes_caller_failure() {
    use crate::ai::{AiState, AlertSoldiersFailureContinuation, Position, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("alert caller has EnemyAi")
        .set_state(AiState::Seeking, Substate::SeekingArrowJustWatching);
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    assert!(
        engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("alert caller has EnemyAi")
            .alert_soldiers(
                Position {
                    x: 100.0,
                    ..Default::default()
                },
                0,
                &global,
                None,
                &ctx,
                &tick,
                AlertSoldiersFailureContinuation::ReturnToDuty,
            ),
        "an admitted candidate suspends the outer AlertSoldiers call"
    );
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("alert recipient has EnemyAi")
        .set_state(AiState::Attacking, Substate::AttackingSwordfight);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("alert caller retains EnemyAi");
    assert!(officer.alerted_us.is_empty());
    assert_eq!(officer.base.current_state, AiState::Default);
    // The all-refused failure resumes ReturnToDuty. The officer already
    // stands on its post facing its initial direction, so the goto-post
    // reach-point fires inside the same synchronous Think tail and the
    // already-facing turn short-circuits straight through GotoPost into
    // OnPost before the drain returns.
    assert_eq!(officer.base.current_substate, Substate::DefaultOnPost);
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.owner == Some(officer_id)
                        && matches!(
                            element.command,
                            crate::element::Command::GatherSoldiers
                                | crate::element::Command::Point
                        )
                })
            })
    );
}

#[test]
fn final_review_alert_partial_refusal_forms_group_from_acceptors_only() {
    use crate::ai::{AiState, AlertSoldiersFailureContinuation, Position, Substate};
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, refused_id, mut assets) = setup_review2_officer_and_soldier();
    let accepted_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(accepted) = engine
        .get_entity_mut(accepted_id)
        .expect("partial alert acceptor exists")
    else {
        panic!("partial alert acceptor changed kind")
    };
    accepted.element.active = true;
    accepted.element.set_position_map(MapPoint::new(0.0, 80.0));
    accepted.npc.life_points = 100;
    let accepted_ai = accepted
        .npc
        .ai_brain
        .enemy_mut()
        .expect("partial alert acceptor has EnemyAi");
    accepted_ai.base.me = accepted_id.index();
    accepted_ai.soldier_profile_rank = ProfileRank::Soldier;
    accepted_ai.set_state(AiState::Default, Substate::DefaultOnPost);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    install_test_open_field_bbox(&mut engine);
    engine
        .get_entity_mut(officer_id)
        .expect("partial alert officer exists")
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    let grid = &engine.world.fast_grid;
    engine
        .world
        .entities
        .get_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial alert caller has EnemyAi")
        .alert_soldiers(
            Position {
                x: 300.0,
                ..Default::default()
            },
            0,
            &global,
            Some(grid),
            &ctx,
            &tick,
            AlertSoldiersFailureContinuation::ReturnToDuty,
        );
    engine
        .get_entity_mut(refused_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial alert rejector has EnemyAi")
        .set_state(AiState::Attacking, Substate::AttackingSwordfight);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("partial alert caller retains EnemyAi");
    assert_eq!(officer.alerted_us, vec![accepted_id.index()]);
    assert_eq!(
        officer.base.current_substate,
        Substate::SeekingOfficerWaitForGroup
    );
    assert!(
        engine
            .get_entity(accepted_id)
            .and_then(Entity::enemy_ai)
            .expect("partial alert acceptor retains EnemyAi")
            .gather_position_instructed
    );
    assert!(
        !engine
            .get_entity(refused_id)
            .and_then(Entity::enemy_ai)
            .expect("partial alert rejector retains EnemyAi")
            .gather_position_instructed
    );
}

#[test]
fn final_review_combat_alert_all_refused_enters_reserve_without_success_remark() {
    use crate::ai::{AiState, Position, Remark, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("combat-alert caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("combat-alert recipient has EnemyAi")
        .set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("combat-alert caller retains EnemyAi");
    assert!(officer.alerted_us.is_empty());
    assert_eq!(officer.base.current_state, AiState::Attacking);
    assert_eq!(officer.base.current_substate, Substate::AttackingReserve);
    assert_ne!(officer.base.current_remark, Remark::OfficerGivesAttackOrder);
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.owner == Some(officer_id)
                        && matches!(
                            element.command,
                            crate::element::Command::GatherSoldiers
                                | crate::element::Command::Point
                        )
                })
            })
    );
}

#[test]
fn command_soldiers_to_attack_does_not_overwrite_acceptor_gather_instruction() {
    use crate::ai::{AiState, Position, Remark, Substate};
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, refused_id, mut assets) = setup_review2_officer_and_soldier();
    let accepted_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(accepted) = engine
        .get_entity_mut(accepted_id)
        .expect("partial-refusal acceptor exists")
    else {
        panic!("partial-refusal acceptor changed kind")
    };
    accepted.element.active = true;
    accepted.element.set_position_map(MapPoint::new(0.0, 80.0));
    accepted.npc.life_points = 100;
    let accepted_ai = accepted
        .npc
        .ai_brain
        .enemy_mut()
        .expect("partial-refusal acceptor has EnemyAi");
    accepted_ai.base.me = accepted_id.index();
    accepted_ai.soldier_profile_rank = ProfileRank::Soldier;
    accepted_ai.set_state(AiState::Default, Substate::DefaultOnPost);
    accepted_ai.gather_direction = 10;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    install_test_open_field_bbox(&mut engine);
    engine
        .get_entity_mut(officer_id)
        .expect("partial-refusal officer exists")
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    assert_eq!(tick.camp_soldiers.len(), 2);
    let global = engine.ai.global.clone();
    let grid = &engine.world.fast_grid;
    engine
        .world
        .entities
        .get_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial-refusal caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            Some(grid),
            &ctx,
            &tick,
        );
    engine
        .get_entity_mut(refused_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial-refusal rejector has EnemyAi")
        .set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal caller retains EnemyAi");
    assert_eq!(officer.alerted_us, vec![accepted_id.index()]);
    assert_eq!(
        officer.base.current_substate,
        Substate::AttackingOfficerGivingOrders
    );
    assert_eq!(officer.base.current_remark, Remark::OfficerGivesAttackOrder);
    let accepted = engine
        .get_entity(accepted_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal acceptor retains EnemyAi");
    assert!(
        !accepted.gather_position_instructed,
        "Original CommandSoldiersToAttack only uses the accepted soldiers to orient the officer; it never assigns formation slots"
    );
    assert_eq!(
        accepted.gather_direction, 10,
        "a combat alert must preserve a direction authored by an independent state such as DoorFight"
    );
    let refused = engine
        .get_entity(refused_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal rejector retains EnemyAi");
    assert!(!refused.gather_position_instructed);
}

#[test]
fn final_review_combat_alert_requires_recipient_360_detection() {
    use crate::ai::Position;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| match entity {
            Entity::Soldier(soldier) => Some(&mut soldier.npc),
            _ => None,
        })
        .expect("360-degree recipient is a soldier")
        .view_radius = 10;
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    let start = engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("360-degree caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    assert_eq!(start, crate::ai_enemy::CommandSoldiersStart::Rejected);
    assert!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::ai_controller)
            .expect("360-degree caller retains AI")
            .outbox
            .reentrant
            .cross_npc_actions
            .is_empty()
    );
}

#[test]
fn closure_review_combat_alert_uses_exact_is_able_to_fight_under_retained_lock() {
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::element::Posture;

    #[derive(Clone, Copy, Debug)]
    enum Ineligible {
        Fleeing,
        Menacing,
        Tied,
        Carried,
        GotHit,
        GotHitStandingUp,
        Hitting,
    }

    let sim = crate::sim_rng::test_context();
    for case in [
        Ineligible::Fleeing,
        Ineligible::Menacing,
        Ineligible::Tied,
        Ineligible::Carried,
        Ineligible::GotHit,
        Ineligible::GotHitStandingUp,
        Ineligible::Hitting,
    ] {
        let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("eligibility recipient exists")
        else {
            panic!("eligibility recipient changed kind")
        };
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("eligibility recipient has EnemyAi")
            .base
            .locks_flag_field = AiLockFlags::BUSY | AiLockFlags::FREEZE;
        match case {
            Ineligible::Fleeing => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Fleeing, Substate::FleeingRunToDoor),
            Ineligible::Menacing => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Menacing, Substate::MenacingPcInComa),
            Ineligible::Tied => soldier.element.posture = Posture::Tied,
            Ineligible::Carried => soldier.human.carrier = Some(officer_id),
            Ineligible::GotHit => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingGotHit),
            Ineligible::GotHitStandingUp => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingGotHitStandingUp),
            Ineligible::Hitting => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingHitting),
        }

        let (start, tick) = start_review_command_soldiers(&mut engine, &sim, &assets, officer_id);
        let candidate = tick
            .camp_soldiers
            .iter()
            .find(|candidate| candidate.handle == soldier_id.index())
            .expect("ineligible active recipient remains represented in camp snapshot");
        assert!(!candidate.is_able_to_fight, "case {case:?}");
        assert_eq!(
            start,
            crate::ai_enemy::CommandSoldiersStart::Rejected,
            "case {case:?} must be rejected before retained-lock Think"
        );
        assert!(
            engine
                .get_entity(officer_id)
                .and_then(Entity::ai_controller)
                .expect("eligibility caller retains AI")
                .outbox
                .reentrant
                .cross_npc_actions
                .is_empty(),
            "case {case:?} must not be called"
        );
    }
}

#[test]
fn closure_review_combat_alert_closed_eyes_do_not_disable_360_detection() {
    use crate::ai::{AiLockFlags, StimulusType};
    use crate::element::EyeStatus;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("closed-eye recipient exists")
    else {
        panic!("closed-eye recipient changed kind")
    };
    soldier.npc.eye_status = EyeStatus::Closed;
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("closed-eye recipient has EnemyAi")
        .base
        .locks_flag_field = AiLockFlags::BUSY;

    let (start, tick) = start_review_command_soldiers(&mut engine, &sim, &assets, officer_id);
    let candidate = tick
        .camp_soldiers
        .iter()
        .find(|candidate| candidate.handle == soldier_id.index())
        .expect("closed-eye recipient is in camp snapshot");
    assert!(candidate.eye_blind);
    assert!(candidate.is_able_to_fight);
    assert_eq!(start, crate::ai_enemy::CommandSoldiersStart::Pending);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    assert_eq!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("closed-eye caller retains EnemyAi")
            .alerted_us,
        vec![soldier_id.index()]
    );
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::ai_controller)
            .expect("closed-eye recipient retains AI")
            .stimulus_queue
            .last()
            .map(|stimulus| stimulus.stimulus_type),
        Some(StimulusType::CallCombatAlert),
        "BUSY retains and accepts the stimulus after eligibility"
    );
}

#[test]
fn closure_review_alert_soldiers_keeps_tied_and_carried_able_to_help() {
    use crate::ai::{AlertSoldiersFailureContinuation, CrossNpcAction, Position};
    use crate::element::Posture;

    let sim = crate::sim_rng::test_context();
    for carried in [false, true] {
        let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("help-eligibility recipient exists")
        else {
            panic!("help-eligibility recipient changed kind")
        };
        if carried {
            soldier.human.carrier = Some(officer_id);
        } else {
            soldier.element.posture = Posture::Tied;
        }

        let (snapshot_able_to_fight, snapshot_able_to_help) =
            engine.test_soldier_snapshot_abilities(&assets, soldier_id);
        assert!(!snapshot_able_to_fight);
        assert!(
            snapshot_able_to_help,
            "Original IsAbleToHelp does not inherit the tied/carried fight gate"
        );

        let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
        let candidate = tick
            .camp_soldiers
            .iter()
            .find(|candidate| candidate.handle == soldier_id.index())
            .expect("tied/carried soldier remains in the owner camp snapshot");
        assert!(!candidate.is_able_to_fight);
        assert!(candidate.is_able_to_help);
        let global = engine.ai.global.clone();
        assert!(
            engine
                .get_entity_mut(officer_id)
                .and_then(Entity::enemy_ai_mut)
                .expect("help-eligibility officer has EnemyAi")
                .alert_soldiers(
                    Position::default(),
                    0,
                    &global,
                    None,
                    &ctx,
                    &tick,
                    AlertSoldiersFailureContinuation::None,
                )
        );
        assert!(matches!(
            engine
                .get_entity(officer_id)
                .and_then(Entity::ai_controller)
                .expect("help-eligibility officer retains AI")
                .outbox
                .reentrant
                .cross_npc_actions
                .as_slice(),
            [CrossNpcAction::RequestThinkResult { target, .. }] if *target == soldier_id.index()
        ));
    }
}

#[test]
fn closure_review_alert_soldiers_keeps_inactive_soldier_in_both_camp_snapshots() {
    use crate::ai::{AlertSoldiersFailureContinuation, CrossNpcAction, Position};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("inactive help recipient exists")
    else {
        panic!("inactive help recipient changed kind")
    };
    soldier.element.active = false;

    let (snapshot_able_to_fight, snapshot_able_to_help) =
        engine.test_soldier_snapshot_abilities(&assets, soldier_id);
    assert!(!snapshot_able_to_fight);
    assert!(
        snapshot_able_to_help,
        "full-tick camp population must use Original IsAbleToHelp's alive/conscious gate"
    );

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let candidate = tick
        .camp_soldiers
        .iter()
        .find(|candidate| candidate.handle == soldier_id.index())
        .expect("inactive soldier remains in the direct-owner camp population");
    assert!(!candidate.is_able_to_fight);
    assert!(candidate.is_able_to_help);

    let global = engine.ai.global.clone();
    assert!(
        engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("inactive-help officer has EnemyAi")
            .alert_soldiers(
                Position::default(),
                0,
                &global,
                None,
                &ctx,
                &tick,
                AlertSoldiersFailureContinuation::None,
            )
    );
    assert!(matches!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::ai_controller)
            .expect("inactive-help officer retains AI")
            .outbox
            .reentrant
            .cross_npc_actions
            .as_slice(),
        [CrossNpcAction::RequestThinkResult { target, .. }] if *target == soldier_id.index()
    ));
}

#[test]
fn closure_review_final_alert_report_boundary_precedes_formation() {
    use crate::ai::{
        AlertSoldiersFailureContinuation, CrossNpcAction, ReportType, Substate,
        ThinkResultContinuation,
    };
    use crate::element::{Detectable, DetectableType, Posture};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let body_id = engine.add_entity(make_test_pc(Posture::Upright));
    let Entity::Pc(body) = engine.get_entity_mut(body_id).expect("report body exists") else {
        panic!("report body changed kind")
    };
    body.element.active = true;
    body.pc.life_points = 0;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    install_test_open_field_bbox(&mut engine);
    engine
        .get_entity_mut(officer_id)
        .expect("final-alert officer exists")
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
    engine
        .get_entity_mut(soldier_id)
        .expect("final-alert recipient exists")
        .npc_data_mut()
        .expect("final-alert recipient is an NPC")
        .detectable_lists[DetectableType::Body as usize]
        .push(Detectable {
            element: Some(body_id),
            detectable_type: DetectableType::Body,
            ..Default::default()
        });
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("final-alert officer has EnemyAi");
        officer.base.my_reconnaissance_report.report_type = ReportType::Body;
        officer
            .base
            .my_reconnaissance_report
            .seen_bodies
            .push(body_id.index());
    }

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = &mut engine.ai.global;
    let grid = &engine.world.fast_grid;
    engine
        .world
        .entities
        .get_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("final-alert officer has EnemyAi")
        .resolve_think_result(
            &sim,
            true,
            soldier_id.index(),
            ThinkResultContinuation::OfficerAlertedSoldier {
                last: true,
                use_formation: true,
                failure: AlertSoldiersFailureContinuation::None,
            },
            global,
            Some(grid),
            &ctx,
            &tick,
        );

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("final-alert officer retains EnemyAi");
    assert!(matches!(
        officer.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [
            CrossNpcAction::ConsiderReport { target, .. },
            CrossNpcAction::FinalizeAlertSoldiers { caller, .. }
        ] if *target == soldier_id.index() && *caller == officer_id.index()
    ));
    assert!(
        officer.base.current_substate != Substate::SeekingOfficerWaitForGroup,
        "formation must remain suspended behind the report boundary"
    );

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let recipient = engine
        .get_entity(soldier_id)
        .expect("final-alert recipient remains present");
    assert!(
        recipient
            .npc_data()
            .expect("final-alert recipient remains an NPC")
            .detectable_lists[DetectableType::Body as usize]
            .iter()
            .all(|detectable| detectable.element != Some(body_id)),
        "ConsiderReport owner effects must close before finalization"
    );
    assert!(
        recipient
            .enemy_ai()
            .expect("final-alert recipient retains EnemyAi")
            .gather_position_instructed,
        "formation resumes after the report boundary"
    );
    assert_eq!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("final-alert officer retains EnemyAi")
            .base
            .current_substate,
        Substate::SeekingOfficerWaitForGroup
    );
}

#[test]
fn get_report_from_soldier_closes_body_deletions_at_owner_boundary() {
    use crate::ai::{AiState, Position, ReportType, Stimulus, StimulusType, Substate};
    use crate::element::{Detectable, DetectableType, Posture};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let mut add_body = || {
        let id = engine.add_entity(make_test_pc(Posture::Lying));
        let Entity::Pc(body) = engine.get_entity_mut(id).expect("report body exists") else {
            panic!("report body changed kind")
        };
        body.element.active = true;
        body.pc.life_points = 0;
        id
    };
    let prefix = add_body();
    let unknown_a = add_body();
    let already_known = add_body();
    let unknown_b = add_body();
    let suffix = add_body();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("report officer has EnemyAi");
        officer.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerWaitForInstructedSoldier,
        );
        officer.base.antagonist = soldier_id.index();
        officer.base.my_reconnaissance_report.report_type = ReportType::Body;
        officer.base.my_reconnaissance_report.seek_position = Position {
            x: 91.0,
            ..Default::default()
        };
        officer.base.my_reconnaissance_report.seen_bodies =
            vec![unknown_a.index(), already_known.index(), unknown_b.index()];
    }
    {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("reporting soldier exists")
        else {
            panic!("reporting entity changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("reporting soldier has EnemyAi");
        ai.base.my_reconnaissance_report.report_type = ReportType::Nothing;
        ai.base.my_reconnaissance_report.seek_position = Position {
            x: 17.0,
            ..Default::default()
        };
        ai.base
            .my_reconnaissance_report
            .seen_bodies
            .push(already_known.index());
        soldier.npc.detectable_lists[DetectableType::Body as usize] =
            [prefix, unknown_a, already_known, unknown_b, suffix]
                .into_iter()
                .map(|id| Detectable {
                    element: Some(id),
                    detectable_type: DetectableType::Body,
                    ..Default::default()
                })
                .collect();
    }

    let report_before = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("reporting soldier retains EnemyAi")
        .base
        .my_reconnaissance_report
        .clone();
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::with_human(StimulusType::CallReport, soldier_id.index()),
        &ctx,
        &tick,
        &assets,
    );

    let recipient = engine
        .get_entity(soldier_id)
        .expect("reporting soldier remains present");
    let body_handles: Vec<_> = recipient
        .npc_data()
        .expect("recipient remains NPC")
        .detectable_lists[DetectableType::Body as usize]
        .iter()
        .map(|detectable| detectable.element.expect("body detectable stays typed"))
        .collect();
    assert_eq!(body_handles, vec![prefix, already_known, suffix]);
    let report_after = &recipient
        .enemy_ai()
        .expect("recipient retains EnemyAi")
        .base
        .my_reconnaissance_report;
    assert_eq!(report_after.report_type, report_before.report_type);
    assert_eq!(report_after.seek_position, report_before.seek_position);
    assert_eq!(report_after.seen_bodies, report_before.seen_bodies);
    assert_eq!(report_after.charly, report_before.charly);
    assert_eq!(report_after.charly_seen, report_before.charly_seen);
}

#[test]
fn review2_alert_result_and_report_finish_before_next_soldier_call() {
    use crate::ai::{
        AlertSoldiersFailureContinuation, CrossNpcAction, Position, ReconnaissanceReport,
        ReportType, StimulusInfo, StimulusType, ThinkResultContinuation,
    };
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let second_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(second) = engine
        .get_entity_mut(second_id)
        .expect("review2 second alerted soldier exists")
    else {
        panic!("review2 second alerted entity changed kind")
    };
    second.element.active = true;
    second.element.set_position_map(MapPoint::new(80.0, 0.0));
    second.npc.life_points = 100;
    let second_ai = second
        .npc
        .ai_brain
        .enemy_mut()
        .expect("review2 second alerted soldier has EnemyAi");
    second_ai.base.me = second_id.index();
    second_ai.soldier_profile_rank = ProfileRank::Soldier;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let first_position = Position {
        x: 10.0,
        ..Default::default()
    };
    let sibling_position = Position {
        x: 20.0,
        ..Default::default()
    };
    let officer = engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 officer has EnemyAi");
    officer.alerted_us.clear();
    officer.base.my_reconnaissance_report.report_type = ReportType::Enemy;
    officer.base.my_reconnaissance_report.seek_position = first_position;
    officer
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestThinkResult {
            target: soldier_id.index(),
            caller: officer_id.index(),
            stimulus_type: StimulusType::CallAlert,
            info: StimulusInfo::Human(officer_id.index()),
            continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                last: false,
                use_formation: false,
                failure: AlertSoldiersFailureContinuation::None,
            },
        });
    officer
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RequestThinkResult {
            target: second_id.index(),
            caller: officer_id.index(),
            stimulus_type: StimulusType::CallAlert,
            info: StimulusInfo::Human(officer_id.index()),
            continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                last: true,
                use_formation: false,
                failure: AlertSoldiersFailureContinuation::None,
            },
        });
    engine
        .get_entity_mut(second_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 second alerted soldier retains AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::ConsiderReport {
            target: soldier_id.index(),
            report: ReconnaissanceReport {
                report_type: ReportType::Enemy,
                seek_position: sibling_position,
                ..Default::default()
            },
            flags: crate::ai_enemy::ReportUpdateFlags::UPDATE_TYPE.bits(),
        });

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    let report = &engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("review2 alerted soldier retains EnemyAi")
        .base
        .my_reconnaissance_report;
    assert_eq!(report.seek_position, sibling_position);
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
fn phalanx_primary_target_propagation_precedes_later_member_assignment() {
    use crate::ai::CrossNpcAction;

    let sim = crate::sim_rng::test_context();
    let (mut engine, source_id, member_id, assets) = setup_review2_officer_and_soldier();
    let propagated_target = 89;
    let member_target = 90;
    let source = engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .expect("phalanx propagation source has AI");
    source.outbox.reentrant.cross_npc_actions.extend([
        CrossNpcAction::SetPrimaryTarget {
            target: member_id.index(),
            primary_target: propagated_target,
        },
        CrossNpcAction::SetPhalanxThemList {
            target: member_id.index(),
            them: vec![member_target],
            primary_target: member_target,
        },
    ]);

    engine.drain_direct_ai_owner_boundary(&sim, source_id, &assets);

    let member = engine
        .get_entity(member_id)
        .and_then(Entity::enemy_ai)
        .expect("phalanx member retains EnemyAi");
    assert_eq!(member.list_them, vec![member_target]);
    assert_eq!(
        member.base.primary_target, member_target,
        "the later inline member decision must win over propagated phalanx target"
    );
    assert!(
        engine
            .get_entity(source_id)
            .and_then(Entity::ai_controller)
            .expect("phalanx propagation source retains AI")
            .outbox
            .reentrant
            .cross_npc_actions
            .is_empty(),
        "the direct target setter must not escape to the next-frame global batch"
    );
}

#[test]
fn phalanx_gather_instruction_skips_a_member_who_already_left_the_formation() {
    use crate::ai::{CrossNpcAction, Position};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let before = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("phalanx gather target has EnemyAi")
        .gather_position;
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("phalanx gather source has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::InstructGatherPosition {
            target: soldier_id.index(),
            position: Position {
                x: 55.0,
                y: 12.0,
                ..Default::default()
            },
            direction: 7,
            call_instruction: true,
        });

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    // The target stands in DefaultOnPost, so the phalanx-correction loop
    // passes over it entirely: neither the slot nor the instruction lands.
    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("phalanx gather target retains EnemyAi");
    assert_eq!(soldier.gather_position, before);
    assert_eq!(soldier.gather_direction, 0);
    assert!(!soldier.gather_position_instructed);
}

fn queue_review2_wrong_kind_think(
    engine: &mut EngineInner,
    officer_id: EntityId,
    civilian_id: EntityId,
    stimulus_type: crate::ai::StimulusType,
    continuation: crate::ai::ThinkResultContinuation,
) {
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 wrong-kind caller has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(crate::ai::CrossNpcAction::RequestThinkResult {
            target: civilian_id.index(),
            caller: officer_id.index(),
            stimulus_type,
            info: crate::ai::StimulusInfo::Human(officer_id.index()),
            continuation,
        });
}

#[test]
#[should_panic(expected = "requires enemy-soldier target")]
fn review2_call_hey_to_civilian_panics_contextually() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, _, mut assets) = setup_review2_officer_and_soldier();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);
    queue_review2_wrong_kind_think(
        &mut engine,
        officer_id,
        civilian_id,
        crate::ai::StimulusType::CallHey,
        crate::ai::ThinkResultContinuation::OfficerCalledSoldier,
    );
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
}

#[test]
#[should_panic(expected = "requires enemy-soldier target")]
fn review2_go_to_officer_to_civilian_panics_contextually() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, _, mut assets) = setup_review2_officer_and_soldier();
    let civilian_id = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);
    queue_review2_wrong_kind_think(
        &mut engine,
        officer_id,
        civilian_id,
        crate::ai::StimulusType::CallGoToOfficer,
        crate::ai::ThinkResultContinuation::OfficerSentCharlyToOfficer,
    );
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
}

#[test]
fn ai_entity_views_keep_inactive_humans_for_same_building_detection() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("inactive snapshot soldier exists")
    else {
        panic!("inactive snapshot entity changed kind")
    };
    soldier.element.active = false;

    let scratch = engine.build_sim_scratch(sim, &LevelAssets::new());
    let view = scratch
        .ai_entity_views
        .get(&soldier_id.index())
        .expect("inactive human must remain available to same-building IsDetecting");
    assert!(!view.active);
}

#[test]
fn messenger_selection_followup_retargets_recording_before_frame_returns() {
    use crate::messenger::{Message, MessageType, PcMessage};

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let second = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    engine.players.seats[0].selection = vec![first];

    // RHMessenger::ForwardMessage handles these calls synchronously.  In
    // particular, SelectCharacter's recursive UpdateRecordingMacro must
    // run before ForwardMessage returns, so the recording target changes
    // in this frame rather than surviving as queued work for the next one.
    engine
        .orders
        .messenger
        .send(Message::pc(PcMessage::StartRecordingMacro, Some(first)));
    engine
        .orders
        .messenger
        .send(Message::pc(PcMessage::SelectCharacter, Some(second)));

    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(engine.players.seats[0].selection, vec![second]);
    assert_eq!(
        engine.players.qa_recording_for,
        vec![second],
        "SelectCharacter -> UpdateRecordingMacro must complete in the originating frame"
    );
    assert!(
        engine
            .orders
            .messenger
            .drain()
            .into_iter()
            .all(|msg| msg.msg_type != MessageType::Pc(PcMessage::UpdateRecordingMacro, None)),
        "the recursive recording update must not remain queued for the next frame"
    );
}
