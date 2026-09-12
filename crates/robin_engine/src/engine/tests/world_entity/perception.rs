use super::*;

#[test]
fn enemy_ai_hero_rejects_soldier_speech_without_invalid_timing_or_stuck_latch() {
    use crate::ai::{AiSpeechAttempt, Remark, SpeechFlags, StimulusType};
    use crate::element::{ActorPc, AiActorData, AiBrain, PcData};

    // Panic is soldier remark 46: the Windows battle recordings attempted
    // Robin group 0x4852002e, which does not exist in the hero voice bank.
    assert_eq!(Remark::Panic as u32, 46);
    for remark in [Remark::Panic, Remark::SeesEnemy, Remark::VipWarcry] {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(Entity::Pc(ActorPc {
            pc: PcData {
                ai: Some(Box::new(AiActorData {
                    ai_brain: AiBrain::Enemy(Box::default()),
                    ..Default::default()
                })),
                ..Default::default()
            },
            element: {
                let mut element = crate::element::ElementData::default();
                element.kind = crate::element::ElementKind::ActorPc;
                element
            },
            actor: Default::default(),
            human: Default::default(),
        }));
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(crate::profiles::CharacterProfile {
                exclamation_id: 0x4852_0000,
                ..Default::default()
            });
        let flags = SpeechFlags::ALWAYS | SpeechFlags::HOUSE | SpeechFlags::MYTALK_1;
        let settlement = engine.settle_npc_speech_attempt(
            &assets,
            owner,
            AiSpeechAttempt {
                remark,
                flags: flags.bits(),
            },
        );
        assert!(settlement.invoke_finished_callback);
        assert_eq!(last_speech_impossible(&engine, owner), Some(11));
        assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
        assert_eq!(exclamation_for(&engine, owner), None);
        let ai = mytalk_ai(&engine, owner);
        assert_eq!(ai.current_remark, remark);
        assert_eq!(ai.outbox.reentrant.self_stimuli.len(), 1);
        assert_eq!(
            ai.outbox.reentrant.self_stimuli[0].stimulus_type,
            StimulusType::EventMyTalk1
        );
        engine.finalize_category_speech_rejection(
            owner,
            settlement
                .category_rejection
                .expect("hero voice category rejection"),
        );
        assert_eq!(
            mytalk_ai(&engine, owner).current_remark,
            Remark::TheSoundOfSilence
        );
        assert_eq!(mytalk_ai(&engine, owner).current_remark_flags, 0);
    }
}

#[test]
fn enemy_ai_hero_speech_completion_clears_enemy_ai_latch() {
    use crate::ai::Remark;
    use crate::element::{ActorPc, AiActorData, AiBrain, ElementData, ElementKind, PcData};

    let mut enemy = crate::ai_enemy::EnemyAi::default();
    enemy.base.current_remark = Remark::Arrow;
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: PcData {
            command_interface: crate::human_control::CommandInterface::None,
            mission_role: crate::human_control::MissionRole::Combatant,
            combat_stance: crate::human_control::CombatStance::Aggressive,
            ai: Some(Box::new(AiActorData {
                ai_brain: AiBrain::Enemy(Box::new(enemy)),
                ..AiActorData::default()
            })),
            ..PcData::default()
        },
    }));
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .push(crate::profiles::CharacterProfile::default());
    engine
        .feedback
        .sound_sim
        .finished_exclamations
        .push((owner.index(), Remark::Arrow as u32));

    engine.settle_npc_speech_completions(&crate::sim_rng::test_context(), &assets);

    assert_eq!(
        engine
            .get_entity(owner)
            .and_then(Entity::ai_controller)
            .expect("AI-controlled hero retains its AI")
            .current_remark,
        Remark::TheSoundOfSilence
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
    let prefix = AiActorOutbox {
        unfocus: true,
        ..Default::default()
    };
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
    let soldier_id = engine.add_test_entity(soldier_entity);

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
        "clearing focus at the recursive actor-effects boundary must leave the caller-tail event behind the older halt"
    );
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(!ai.outbox.actor.halt);
}

#[test]
fn friendly_alert_soldier_tail_does_not_extend_end_think() {
    let mut ai = crate::ai::AiController::new(168);
    ai.think_recursion_depth = 1;
    ai.completion_latch_inside_think = true;
    ai.outbox.reentrant.alert_soldier_completion_pending = true;

    // Soldier alerting's result and optional retry are fully consumed by its
    // owner-work continuation. Unlike corpse-alert processing, it does not author a
    // second route which needs the original decision frame to remain open.
    assert!(ai.end_think_completion_events());
    assert_eq!(ai.think_recursion_depth, 0);
    assert_eq!(ai.engine_deferred_end_think_frames, 0);
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
        identifier: u32::from(Remark::Wounded as u16),
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
            enemy.base.friend_in_trouble = None;
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
        assert_eq!(
            enemy.base.friend_in_trouble,
            Some(crate::ai::AiEntityHandle::new(charly_handle))
        );
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
    let owner = early.add_test_entity(entity);
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
    let owner = engine.add_test_entity(entity);
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
fn alert_soldier_typed_tail_owns_couldnt_reachpoint_before_event_surface() {
    use crate::element::AiBrain;

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(owner)
        .expect("soldier-alert test civilian exists")
    else {
        panic!("soldier-alert test owner changed kind")
    };
    civilian.npc.ai_brain =
        AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(owner.index())));
    let ai = civilian
        .npc
        .ai_brain
        .base_mut()
        .expect("soldier-alert test civilian has AI");
    ai.completion_latch_inside_think = true;
    ai.couldnt_reachpoint = true;
    ai.outbox.reentrant.alert_soldier_completion_pending = true;

    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("soldier-alert test civilian retains AI");
    assert!(ai.couldnt_reachpoint);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());

    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("soldier-alert test civilian retains mutable AI")
        .outbox
        .reentrant
        .alert_soldier_completion_pending = false;
    engine.surface_synchronous_completion_events_for_owner(owner);
    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("soldier-alert test civilian retains AI after surface");
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
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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

    // The generic tick-completion boundary must leave officer alerting's synchronous
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
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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

    // Tick completion observes the typed latch before the owner continuation runs.
    engine.surface_synchronous_completion_events_for_owner(owner);
    engine.drain_ai_owner_work_for(&sim, &assets, owner);
    engine.drain_direct_ai_owner_boundary_mode(
        &sim,
        owner,
        &assets,
        crate::engine::ai::OwnerBoundaryPolicy::WithoutForecast,
    );

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
#[should_panic(expected = "non-enemy AI brain")]
fn dead_body_alert_tail_fails_loud_for_wrong_ai_owner() {
    use crate::ai::{AiOwnerWork, Position};
    use crate::element::AiBrain;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
        .expect("soldier-alert owner has AI");
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
        .expect("soldier-alert owner retains AI");
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
    let soldier = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for (id, x) in [(owner, 0.0), (soldier, 100.0)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("soldier-alert route actor exists");
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
        .expect("soldier-alert owner has AI");
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
        .expect("soldier-alert owner retains AI");
    assert_eq!(
        ai.antagonist,
        Some(crate::ai::AiEntityHandle::new(soldier.index()))
    );
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
        .expect("soldier-alert owner has AI");
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

    // Soldier alerting and its caller's panic tail are one synchronous original-game
    // call stack. Exercise the complete direct-owner boundary so the staged
    // `begin_panic` request performs its engine-owned door lookup and sole
    // final state change before asserting the outcome.
    engine.drain_direct_ai_owner_boundary_mode(
        &sim,
        owner,
        &assets,
        crate::engine::ai::OwnerBoundaryPolicy::WithoutForecast,
    );

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("soldier-alert owner retains AI");
    assert_eq!(ai.current_state, AiState::Fleeing);
    assert!(!ai.couldnt_reachpoint);
    assert!(!ai.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.outbox.reentrant.owner_work.is_empty());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn alert_soldier_friend_append_drain_preserves_preexisting_duplicate_and_order() {
    use crate::element::{AiBrain, Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let first_friend = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let second_friend = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));

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
    // This is the exact duplicate-preserving path used by soldier alerting's
    // direct retail detectable additions.
    ai.outbox.actor.detectable_mutations.extend([
        crate::ai::DetectableMutation::Append(first_friend, DetectableType::Friend),
        crate::ai::DetectableMutation::Append(second_friend, DetectableType::Friend),
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
        "the existing friend must be duplicated and the soldier-alert registry order retained"
    );
}

#[test]
fn detectable_enemy_add_filters_targets_but_append_preserves_direct_calls() {
    use crate::ai::DetectableMutation::{Add, Append};
    use crate::element::{
        Camp, DetectableType::Enemy, ElementBonus, ElementData, ElementKind, ObjectData, Posture,
    };
    let sim = crate::sim_rng::test_context();
    for (camp, target_entity, accepted) in [
        (
            Camp::Lacklandists,
            make_test_soldier(Posture::Upright),
            false,
        ),
        (Camp::Royalists, make_test_soldier(Posture::Upright), true),
        (Camp::Royalists, make_test_pc(Posture::Upright), false),
        (Camp::Lacklandists, make_test_pc(Posture::Upright), true),
        (
            Camp::Lacklandists,
            Entity::Bonus(ElementBonus {
                element: {
                    let mut element = ElementData::default();
                    element.kind = ElementKind::ObjectBonus;
                    element
                },
                object: ObjectData::default(),
            }),
            false,
        ),
    ] {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(camp));
        let target = engine.add_test_entity(target_entity);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap()
            .outbox
            .actor
            .detectable_mutations = vec![Add(target, Enemy), Append(target, Enemy)];
        engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());
        let entries = &engine
            .get_entity(owner)
            .unwrap()
            .ai_actor_data()
            .unwrap()
            .detectable_lists[Enemy as usize];
        assert_eq!(
            entries.len(),
            if accepted { 2 } else { 1 },
            "owner {camp:?}, target {target:?}"
        );
        assert!(entries.iter().all(|entry| entry.element == Some(target)));
    }
}

#[test]
#[should_panic(expected = "detectable target 999 disappeared")]
fn detectable_enemy_add_requires_a_live_target() {
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .actor
        .add_detectable((
            EntityId::Soldier(crate::entity_id::SoldierId(999)),
            crate::element::DetectableType::Enemy,
        ));
    engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());
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
    let scratch = accepted.build_sim_scratch(&assets);
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
            &assets.navigation.hiking_paths,
            &assets.navigation.hiking_waypoint_sectors,
            &accepted.ai.global.all_soldier_handles,
            accepted.control.sim_config.difficulty,
        )
    };
    let civilian_entity_id = EntityId::Civilian(civilian_id);
    let tick = accepted.build_npc_tick_data(&sim_context, civilian_entity_id, &assets);
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
    // AI human handles use zero as missing, so keep production NPCs off slot 0.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let soldier_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
        .antagonist = Some(crate::ai::AiEntityHandle::new(soldier_id.index()));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let scratch = engine.build_sim_scratch(&assets);
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
        &assets.navigation.hiking_paths,
        &assets.navigation.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, officer_id, &assets);
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
    let officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
    let scratch = engine.build_sim_scratch(&assets);
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
        &assets.navigation.hiking_paths,
        &assets.navigation.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, officer_id, &assets);
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
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let reporter_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let callback_officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
            info: crate::ai::StimulusInfo::Human(crate::ai::AiEntityHandle::new(
                callback_officer_id.index(),
            )),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    let scratch = engine.build_sim_scratch(&assets);
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
            &assets.navigation.hiking_paths,
            &assets.navigation.hiking_waypoint_sectors,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    let tick = engine.build_npc_tick_data(sim, reporter_id, &assets);
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
        Some(crate::ai::AiEntityHandle::new(callback_officer_id.index())),
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
        reporter.base.antagonist = Some(crate::ai::AiEntityHandle::new(officer_id.index()));
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
        "the continuation's state-change callback escaped the direct decision-tick boundary"
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
        engine.add_test_entity(entity)
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
    // The close side-on special case succeeds before view-radius/LOS calculation.
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
        enemy.base.my_reconnaissance_report.charly =
            Some(crate::ai::AiEntityHandle::new(charly.index()));
        enemy.base.checkpoint_charly = Some(crate::ai::AiEntityHandle::new(charly.index()));
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
        owner_ai.base.antagonist =
            Some(crate::ai::AiEntityHandle::new(excluded_antagonist.index()));
        owner_ai
            .base
            .outbox
            .actor
            .queue_unalert_near_charly_seekers(
                CharlySeekerTarget::Npc(crate::ai::AiEntityHandle::new(charly.index())),
                owner_ai.base.antagonist,
            );
        // Model rejected speech returning to duty after the original-game action
        // but before Rust drains the engine-side sweep.
        owner_ai.base.antagonist = None;
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
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
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
        assert_eq!(enemy.base.checkpoint_charly, None);
        assert_eq!(enemy.base.sorrow_level, 0);
        assert!(
            soldier.npc.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
            "clearing the checkpoint friend must synchronously clear the recipient's missed-friend list"
        );
        assert!(
            enemy
                .base
                .outbox
                .actor
                .deleted_detectable_types()
                .is_empty()
        );
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
    // The all-refused failure resumes returning to duty. The officer already
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
    let accepted_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
fn search_charly_caller_timer_follows_deferred_alert_finalization() {
    use crate::ai::{AiState, AlertSoldiersFailureContinuation, Substate};

    let sim = crate::sim_rng::test_context();
    for suspended_substate in [
        Substate::DefaultLookingForCharly,
        Substate::DefaultLookingSidewardsForCharly,
    ] {
        let (mut engine, officer_id, _soldier_id, assets) = setup_review2_officer_and_soldier();
        engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("alert caller has EnemyAi")
            .set_state(AiState::Default, suspended_substate);
        let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
        let global = engine.ai.global.clone();
        {
            let officer = engine
                .get_entity_mut(officer_id)
                .and_then(Entity::enemy_ai_mut)
                .expect("alert caller has EnemyAi");
            assert!(officer.alert_soldiers(
                ctx.position,
                0,
                &global,
                None,
                &ctx,
                &tick,
                AlertSoldiersFailureContinuation::SeekMissedCharly {
                    center: ctx.position,
                },
            ));
            // This is the caller tail authored immediately after missing-PC search
            // in DEFAULT_LOOKING_FOR_CHARLY. Its random-look prelude can have
            // changed the suspended substate to the sidewards variant.
            officer.base.launch_timer(
                crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32,
                ctx.frame,
            );
        }

        engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

        let officer = engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("alert caller retains EnemyAi");
        assert_eq!(
            officer.base.current_substate,
            Substate::SeekingOfficerWaitForGroup
        );
        assert_eq!(
            officer.base.when_does_timer_ring,
            ctx.frame
                .wrapping_add(crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32),
            "the synchronous caller's timer write must follow AlertSoldiers' 20-frame timer"
        );
        assert_eq!(
            officer.base.substate_at_last_timer_launch,
            Substate::SeekingOfficerWaitForGroup,
            "the trailing timer is authored after AlertSoldiers changes the state"
        );
    }
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
            soldier.element.publish_order_posture(Posture::Tied);
        }

        let (snapshot_able_to_fight, snapshot_able_to_help) =
            engine.test_soldier_snapshot_abilities(&assets, soldier_id);
        assert!(!snapshot_able_to_fight);
        assert!(
            snapshot_able_to_help,
            "help eligibility does not include the tied/carried fight gate"
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
fn closure_review_final_alert_report_boundary_precedes_formation() {
    use crate::ai::{
        AlertSoldiersFailureContinuation, CrossNpcAction, ReportType, Substate,
        ThinkResultContinuation,
    };
    use crate::element::{Detectable, DetectableType, Posture};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let body_id = engine.add_test_entity(make_test_pc(Posture::Upright));
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
fn review2_alert_result_and_report_finish_before_next_soldier_call() {
    use crate::ai::{
        AlertSoldiersFailureContinuation, CrossNpcAction, Position, ReconnaissanceReport,
        ReportType, StimulusInfo, StimulusType, ThinkResultContinuation,
    };
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let second_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
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
            info: StimulusInfo::Human(crate::ai::AiEntityHandle::new(officer_id.index())),
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
            info: StimulusInfo::Human(crate::ai::AiEntityHandle::new(officer_id.index())),
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
#[should_panic(expected = "requires enemy-soldier target")]
fn review2_call_hey_to_civilian_panics_contextually() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, _, mut assets) = setup_review2_officer_and_soldier();
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
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
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
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
