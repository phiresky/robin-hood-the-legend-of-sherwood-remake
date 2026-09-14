use super::*;
use crate::element::Human as _;

#[test]
fn live_state_changes_preserve_formation_links_then_clear_both_reciprocals() {
    use crate::ai::{AiEntityHandle, AiState, Substate};
    let mut engine = EngineInner::new();
    engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let left = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let right = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOverviewLookLeft;
        ai.left_combat_neighbour = Some(AiEntityHandle::new(left.index()));
        ai.right_combat_neighbour = Some(AiEntityHandle::new(right.index()));
    }
    engine
        .get_entity_mut(left)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .right_combat_neighbour = Some(AiEntityHandle::new(owner.index()));
    engine
        .get_entity_mut(right)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .left_combat_neighbour = Some(AiEntityHandle::new(owner.index()));
    let sim = crate::sim_rng::test_context();
    engine.duty_set_state(
        &sim,
        &assets,
        owner,
        AiState::Attacking,
        Substate::AttackingRunningToPhalanx,
    );
    let ai = engine.get_entity(owner).and_then(Entity::enemy_ai).unwrap();
    assert_eq!(
        ai.left_combat_neighbour,
        Some(AiEntityHandle::new(left.index()))
    );
    assert_eq!(
        ai.right_combat_neighbour,
        Some(AiEntityHandle::new(right.index()))
    );
    engine.duty_set_state(
        &sim,
        &assets,
        owner,
        AiState::Attacking,
        Substate::AttackingOverviewLookLeft,
    );
    let ai = engine.get_entity(owner).and_then(Entity::enemy_ai).unwrap();
    assert_eq!(ai.left_combat_neighbour, None);
    assert_eq!(ai.right_combat_neighbour, None);
    assert_eq!(
        engine
            .get_entity(left)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .right_combat_neighbour,
        None
    );
    assert_eq!(
        engine
            .get_entity(right)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .left_combat_neighbour,
        None
    );
}

#[test]
fn live_state_change_releases_archery_ownership_without_clearing_special_strike() {
    use crate::ai::{AiState, PointArchery, SectorArchery, Substate};
    use crate::sector::{ArcheryPointIdx, SectorNumber};
    let mut engine = EngineInner::new();
    engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.ai.global.archery_sectors.push(SectorArchery {
        points: vec![PointArchery {
            position: Default::default(),
            direction: 0,
            is_shooting_point: true,
            sector_index: SectorNumber::new(1),
            owner: Some(owner),
        }],
        polygon: Vec::new(),
        layer: 0,
        index_first_shooting_point: Some(ArcheryPointIdx(0)),
        index_last_shooting_point: Some(ArcheryPointIdx(0)),
        num_shooting_points: 1,
        num_owners: 1,
    });
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .unwrap();
    ai.my_shooting_point = Some((0, 0));
    ai.my_archery_sector = Some(0);
    ai.pending_special_strike = true;
    engine.duty_set_state(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        AiState::Default,
        Substate::DefaultOnPost,
    );
    let ai = engine.get_entity(owner).and_then(Entity::enemy_ai).unwrap();
    assert_eq!(ai.my_shooting_point, None);
    assert_eq!(ai.my_archery_sector, None);
    assert!(ai.pending_special_strike);
    assert_eq!(engine.ai.global.archery_sectors[0].points[0].owner, None);
    assert_eq!(engine.ai.global.archery_sectors[0].num_owners, 0);
}

#[test]
fn removal_revalidates_stimuli_detached_across_a_synchronous_boundary() {
    use crate::ai::{AiEntityHandle, Stimulus, StimulusInfo, StimulusType};

    let mut engine = EngineInner::new();
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let observer = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let mut human = Stimulus::new(StimulusType::EventDone);
    human.info = StimulusInfo::Human(AiEntityHandle::new(target.index()));
    let mut object = human;
    object.info = StimulusInfo::Object(AiEntityHandle::new(target.index()));
    // A synchronous caller can hold this batch while a nested call removes
    // the target. Removal cannot edit the caller's local snapshot.
    let detached = vec![human, object];
    engine.remove_entity(target);
    let ai = engine
        .get_entity_mut(observer)
        .unwrap()
        .ai_controller_mut()
        .unwrap();
    let history = ai.last_stimulus;
    ai.outbox.detection.stimuli = detached;
    engine.tick_enemy_ai_drain_pending_stimuli_for_npc(
        &crate::sim_rng::test_context(),
        observer,
        &LevelAssets::new(),
    );
    let ai = engine
        .get_entity(observer)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(ai.outbox.detection.stimuli.is_empty());
    assert_eq!(
        ai.last_stimulus, history,
        "neither stale target is delivered to Think"
    );
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
fn geometry_only_level_reserves_zero_ai_handle_before_first_actor() {
    let mut engine = EngineInner::new();
    engine.reserve_null_ai_handle_slot_if_empty();

    assert_eq!(engine.world.entities.len(), 1);
    assert!(engine.world.entities.get_legacy_slot(0).is_none());

    let soldier = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Custom(2)));
    assert_eq!(soldier.index(), 1, "the first AI actor must not alias null");
    assert_eq!(
        engine
            .get_entity(soldier)
            .and_then(Entity::enemy_ai)
            .expect("test soldier has enemy AI")
            .base
            .me,
        1,
        "published AI self handle follows the reserved entity slot"
    );
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
        super::super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, frame);
        engine.settle_npc_speech_completions(&sim, &assets);
        let ai = mytalk_ai(&engine, soldier_id);
        assert_eq!(ai.current_remark, Remark::Arrow);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    engine.control.frame_counter = 103;
    super::super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
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
    super::super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
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
fn pre_set_state_face_and_attentive_leave_register_then_preempt_in_manager_fifo() {
    use crate::ai::{AiActorOutbox, AiOwnerWork, AiState, AttentiveModeEffect, Substate};
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
    let owner = engine.add_test_entity(soldier_entity);
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
            .push(AiOwnerWork::ActorEffects(face_prefix));
    }
    engine.duty_set_state(
        &sim,
        &assets,
        owner,
        AiState::Default,
        Substate::DefaultGotoPostTurn,
    );
    {
        let ai = engine
            .get_entity_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        ai.outbox
            .actor
            .queue_set_attentive_mode(AttentiveModeEffect::new(false, false));
    }

    // This is the movement-condolation mode that exposed the bug. Face and
    // attentive-mode changes both launch inline, but their ordinary
    // elements remain registered until the global sequence-manager update.
    engine.drain_direct_ai_owner_boundary(&sim, owner, &assets);

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
        "facing must register before the state change's attentive tail without instructing either in the owner slot"
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
        "attentive-mode changes update their gate immediately"
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
    let owner = engine.add_test_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.duty_set_state(
        &sim,
        &assets,
        owner,
        AiState::Attacking,
        Substate::AttackingReactiontime,
    );
    engine.duty_set_state(
        &sim,
        &assets,
        owner,
        AiState::Attacking,
        Substate::AttackingTooProudToAttackApproach,
    );

    engine.drain_direct_ai_owner_boundary(&sim, owner, &assets);

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
    let owner = engine.add_test_entity(soldier_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.drain_direct_ai_owner_boundary(&sim, owner, &assets);

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
        "the original game launches both opposite attentive-mode transitions synchronously before the following facing step"
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
        "remark-completion notification observes the rejected speech latch"
    );

    let recursive = engine.settle_npc_speech_attempt(
        &assets,
        owner,
        AiSpeechAttempt {
            remark: Remark::Arrow,
            flags: (SpeechFlags::ALWAYS | SpeechFlags::EMERGENCY).bits(),
        },
    );
    assert_eq!(
        recursive,
        super::super::super::ai::NpcSpeechSettlement::default()
    );
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
fn repeated_checkpoint_charly_drains_only_the_last_target() {
    use crate::ai::AiEntityHandle;
    use crate::element::DetectableType::MissedFriend;
    let sim = crate::sim_rng::test_context();
    for clear in [false, true] {
        let mut engine = EngineInner::new();
        let owner =
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let first = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
        let second = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
        let ai = engine
            .get_entity_mut(owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap();
        ai.set_checkpoint_charly(Some(AiEntityHandle::new(first.index())));
        ai.set_checkpoint_charly((!clear).then_some(AiEntityHandle::new(second.index())));
        engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());
        let actual = engine
            .get_entity(owner)
            .unwrap()
            .ai_actor_data()
            .unwrap()
            .detectable_lists[MissedFriend as usize]
            .iter()
            .map(|entry| entry.element)
            .collect::<Vec<_>>();
        assert_eq!(actual, if clear { vec![] } else { vec![Some(second)] });
    }
}

#[test]
fn fighter_registry_keeps_inactive_and_tied_members_with_live_ineligibility() {
    use crate::element::Posture;

    let mut engine = EngineInner::new();
    let self_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let other_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

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
    other_soldier.element.publish_order_posture(Posture::Tied);

    let registry = engine.world.fighter_registry_order();
    assert!(registry.contains(&self_id));
    assert!(registry.contains(&other_id));
    for id in [self_id, other_id] {
        let Entity::Soldier(soldier) = engine.get_entity(id).expect("registered fighter") else {
            panic!("fighter changed kind")
        };
        assert!(!soldier.is_able_to_fight());
        assert!(!engine.get_entity(id).unwrap().is_dead());
        assert!(!soldier.human.unconscious);
    }
}

#[test]
fn full_fighter_registry_retains_dead_pc_for_held_ai_targets() {
    let mut engine = EngineInner::new();
    let self_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let dead_pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));

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

    assert!(engine.world.fighter_registry_order().contains(&dead_pc_id));
    let entity = engine
        .get_entity(dead_pc_id)
        .expect("held dead fighter remains live");
    assert!(entity.is_dead());
    let Entity::Pc(pc) = entity else {
        panic!("dead fighter changed kind")
    };
    assert!(!pc.is_able_to_fight());
}

#[test]
fn filtered_think_refreshes_live_friend_primary_target_for_battle_decisions() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::MapPoint;

    let sim = crate::sim_rng::test_context();
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
    let owner_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let friend_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let new_target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

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
        {
            let ai = &mut enemy.base;
            ai.set_ai_state(AiState::Attacking);
            ai.current_substate = substate;
        }
        enemy.base.primary_target = Some(crate::ai::AiEntityHandle::new(old_target_id.index()));
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
    let friend = engine
        .get_entity_mut(friend_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("friend has Enemy AI");
    friend.base.primary_target = Some(crate::ai::AiEntityHandle::new(new_target_id.index()));
    engine.dispatch_think_with_drain(
        &sim,
        owner_id,
        &Stimulus::new(StimulusType::EventTimer),
        None,
        &assets,
    );

    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("owner retains Enemy AI");
    assert!(owner.list_them.contains(&new_target_id.index()));
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

#[test]
fn officer_call_rejection_closes_return_to_duty_actor_fixed_point() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::{Command, Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    engine.drain_direct_ai_owner_boundary(&sim, soldier_id, &assets);

    let sector = Some(crate::engine::test_support::ensure_ordinary_sector(
        &mut engine,
        1,
        0,
    ));
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
    officer_ai.base.antagonist = Some(crate::ai::AiEntityHandle::new(soldier_id.index()));
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
    {
        let ai = &mut engine
            .get_entity_mut(soldier_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("call rejector has EnemyAi")
            .base;
        ai.set_ai_state(AiState::Attacking);
        ai.current_substate = Substate::AttackingSwordfight;
    }

    assert_eq!(
        engine.execute_ai_officer_rendezvous_event(
            &sim,
            &assets,
            officer_id,
            &Stimulus::new(StimulusType::EventDone)
        ),
        Some(false),
    );

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
        "leaving attentive mode must precede movement: {commands:?}"
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
fn live_return_to_duty_publishes_goto_after_attentive_inline() {
    use crate::ai::{AiState, DutyFlags, Substate};
    use crate::coordinates::MapPoint;
    use crate::element::Command;
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::gate::Door;
    use crate::sector::{SectorNumber, SectorType};

    let make_sector = |sector_type| GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type,
        layer: 0,
        sector_number: SectorNumber::new(64),
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

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = crate::coordinates::MapSize::new(500.0, 500.0);
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "adopted_return_to_duty_boundary_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed duplicate-public gate"),
    );
    {
        let level = engine.world.fast_grid_mut().level_mut();
        level.sectors = vec![
            make_sector(SectorType::MOTION | SectorType::AREA | SectorType::BUILDING),
            make_sector(SectorType::MOTION | SectorType::AREA),
        ];
        level.sector_number_map.insert(SectorNumber::new(64), 1);
    }
    engine.script_domains.interactables.doors = vec![Door {
        point_out: MapPoint::new(160.0, 100.0),
        point_in: MapPoint::new(120.0, 100.0),
        sector_out: SectorNumber::new(64),
        sector_in: SectorNumber::new(64),
        sector_out_index: SectorIndex::new(1),
        sector_in_index: SectorIndex::new(0),
        ..Door::default()
    }];
    crate::gate::build_gate_links(&mut engine.script_domains.interactables.doors);

    let source = crate::position_interface::SectorHandle::new(64)
        .unwrap()
        .with_arena_index(SectorIndex::new(0).unwrap());
    let goal = crate::legacy_save::adopt_elements::adopt_position_sector_for_test(
        vec![
            crate::position_interface::SectorHandle::new(64),
            crate::position_interface::SectorHandle::new(64),
        ],
        vec![SectorIndex::new(0), SectorIndex::new(1)],
        1,
    )
    .expect("saved initial-position sector resolves");
    assert_eq!(goal.arena_index(), SectorIndex::new(1));
    assert_eq!(
        crate::legacy_save::adopt_elements::adopt_position_sector_for_test(
            vec![crate::position_interface::SectorHandle::new(64)],
            vec![None],
            0,
        ),
        crate::position_interface::SectorHandle::new(64),
        "a genuinely missing retained index remains number-only"
    );
    let entity = engine.get_entity_mut(owner).unwrap();
    entity.element_data_mut().active = true;
    entity
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    entity
        .element_data_mut()
        .sprite
        .position_iface
        .set_sector_topology(Some(source), source.arena_index());
    let ai = entity.enemy_ai_mut().unwrap();
    ai.base.me = owner.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGetInstructedByOfficer;
    ai.base.initial_position = crate::ai::Position {
        x: 300.0,
        y: 100.0,
        sector: Some(goal),
        level: 0,
    };
    ai.attentive = true;
    ai.will_be_attentive = true;

    engine.execute_ai_return_to_duty(&sim, &assets, owner, DutyFlags::empty());
    let commands = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner))
        .map(|element| element.command)
        .collect::<Vec<_>>();
    assert_eq!(commands.first(), Some(&Command::LeaveAttentiveMode));
    assert!(
        commands.contains(&Command::Move),
        "different exact arenas with the same public number can only publish Move after the gate graph accepts the route"
    );
    assert!(
        !engine
            .get_entity(owner)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .base
            .couldnt_reachpoint
    );
}

#[test]
fn officer_call_acceptance_keeps_wait_state_timer_and_beggar() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
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
    officer_ai.base.antagonist = Some(crate::ai::AiEntityHandle::new(soldier_id.index()));
    officer_ai.attentive = true;
    officer_ai.will_be_attentive = true;
    officer_ai.base.current_state = AiState::Seeking;
    officer_ai.base.current_substate = Substate::SeekingOfficerCallSoldier;

    assert_eq!(
        engine.execute_ai_officer_rendezvous_event(
            &sim,
            &assets,
            officer_id,
            &Stimulus::new(StimulusType::EventDone)
        ),
        Some(false),
    );

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
    assert_eq!(
        ai.base.antagonist,
        Some(crate::ai::AiEntityHandle::new(soldier_id.index()))
    );
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
    assert_eq!(
        soldier.base.antagonist,
        Some(crate::ai::AiEntityHandle::new(officer_id.index()))
    );
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
    target_ai.base.antagonist = Some(crate::ai::AiEntityHandle::new(source_id.index()));

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
            info: StimulusInfo::Human(crate::ai::AiEntityHandle::new(source_id.index())),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    engine.drain_direct_ai_owner_boundary(&sim, source_id, &assets);

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
fn recursive_break_phalanx_preserves_enclosing_think_without_owning_end_think() {
    use crate::ai::{AiState, StimulusType, Substate};
    use crate::element::Camp;

    let sim = crate::sim_rng::test_context();
    let (mut engine, source_id, member_id, assets) = setup_review2_officer_and_soldier();
    engine.enter_ai_think_frame(source_id);
    let Entity::Soldier(source_soldier) = engine
        .get_entity_mut(source_id)
        .expect("phalanx-break source exists")
    else {
        panic!("phalanx-break source changed kind")
    };
    // Keep battle planning on its active-enemy path; the empty synthetic
    // patrol fixture otherwise recurses through unrelated patrol setup.
    source_soldier.soldier.cached_camp = Camp::Royalists;
    {
        let member = engine
            .get_entity_mut(member_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("phalanx-break member has EnemyAi");
        member.base.current_state = AiState::Attacking;
        member.base.current_substate = Substate::AttackingPhalanx;
        member.list_them = vec![source_id.index()];
        member.base.primary_target = Some(crate::ai::AiEntityHandle::new(source_id.index()));
        // Model a completion candidate already present on the recursively
        // called object. Phalanx breaking has no matching tick completion of its own,
        // so its engine prefix must not dispatch the flag.
        member.base.already_on_point = true;
    }

    engine.execute_ai_break_phalanx(&sim, &assets, member_id, false, false);

    let member = engine
        .get_entity(member_id)
        .and_then(Entity::enemy_ai)
        .expect("phalanx-break member retains EnemyAi");
    assert_eq!(
        engine.ai.think_call_stack,
        vec![source_id],
        "the direct member call must preserve the enclosing decision frame"
    );
    assert!(
        !member
            .base
            .outbox
            .reentrant
            .self_stimuli
            .iter()
            .any(|queued| queued.stimulus_type == StimulusType::EventReachPoint),
        "the recursively called member owns no decision-completion surface"
    );
}

#[test]
fn phalanx_gather_instruction_skips_a_member_who_already_left_the_formation() {
    use crate::ai::Position;

    let sim = crate::sim_rng::test_context();
    let (mut engine, _officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let before = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("phalanx gather target has EnemyAi")
        .gather_position;
    engine.instruct_live_phalanx(
        &sim,
        &assets,
        &[soldier_id],
        Position {
            x: 55.0,
            y: 12.0,
            ..Default::default()
        },
        crate::coordinates::MapVec::new(25.0, 0.0),
        7,
    );

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

#[test]
fn messenger_selection_followup_retargets_recording_before_frame_returns() {
    use crate::messenger::{Message, MessageType, PcMessage};

    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let second = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    engine.players.seats[0].selection = vec![first];

    // Original-game message forwarding handles these calls synchronously. In
    // particular, character selection's recursive macro-recording update must
    // run before message forwarding returns, so the recording target changes
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
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    assert_eq!(engine.players.seats[0].selection, vec![second]);
    assert_eq!(
        engine.players.qa_recording_for,
        vec![second],
        "character selection followed by macro-recording update must complete in the originating frame"
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
