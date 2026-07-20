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

fn build_mytalk_timing_test(duration_frames: Option<u32>) -> (EngineInner, EntityId, LevelAssets) {
    use crate::ai::{Remark, SpeechFlags};
    use crate::element::AiBrain;
    use crate::profiles::SoldierProfile;
    use crate::sound::ExclamationGroup;

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
    if let Some(frames) = duration_frames {
        std::sync::Arc::make_mut(&mut assets.exclamation_durations).insert(
            (
                ExclamationGroup::Civilian,
                SPEECH_TIMING_PROFILE_ID,
                Remark::Arrow as u16,
            ),
            frames,
        );
    }

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

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(Some(3));
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);

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
fn missing_exclamation_duration_completes_mytalk_at_next_boundary() {
    use crate::ai::{LogLineType, Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(None);
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame, 100,
        "missing metadata must not fabricate a 75-frame speech"
    );
    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(ai.current_remark, Remark::Arrow);

    engine.control.frame_counter = 101;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 101);
    engine.settle_npc_speech_completions(&sim, &assets);

    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(engine.control.frame_counter, 101);
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

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(Some(3));
    let sim = crate::sim_rng::test_context();
    engine.drain_ai_owner_work_for(&sim, &assets, soldier_id);
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

    let fighters = engine.build_nearby_fighters_for(self_id, &assets);
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
