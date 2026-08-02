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
            .playing_exclamations
            .iter()
            .any(|line| line.actor_id == owner.index()
                && line.exclamation_id == Remark::Arrow as u32),
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
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: owner.index() as u16,
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
            guy_index: owner.index() as u16,
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
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: first.index() as u16,
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
    early.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: owner.index() as u16,
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
        guy_index: first.index() as u16,
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
    engine.world.fast_grid.level = std::sync::Arc::new(level);
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
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(pc),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    assert!(engine.pc_has_pending_shoot_bow(pc));

    let _ = engine.enter_swordfight(sim, &LevelAssets::new(), pc, opponent, false);

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

fn run_synchronous_charly_report(officer_state: crate::ai::AiState) -> EngineInner {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::EyeStatus;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.world.weather.ambiance = crate::engine::types::Ambiance::Night;
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
            .push(trigger);
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
    assert_eq!(officer.base.current_substate, Substate::DefaultGotoPost);
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
fn final_review_combat_alert_partial_refusal_uses_only_acceptor_for_formation() {
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
    complete_test_runtime_fixture(&mut engine, &mut assets);
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
    assert!(accepted.gather_position_instructed);
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
