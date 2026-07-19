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
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.current_remark = Remark::Arrow;
    ai.current_remark_flags = (SpeechFlags::MYTALK_1 | SpeechFlags::ALWAYS).bits();
    let soldier_id = engine.add_entity(soldier_entity);

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(SoldierProfile {
            profile_name: "timing-test-soldier".into(),
            exclamation_id: SPEECH_TIMING_PROFILE_ID,
            ..Default::default()
        });
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
    use crate::ai::{Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(Some(3));
    engine.process_npc_speech(&assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame,
        103
    );
    assert!(mytalk_ai(&engine, soldier_id).speech_in_flight);

    for frame in [101, 102] {
        engine.control.frame_counter = frame;
        super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, frame);
        engine.process_npc_speech(&assets);
        let ai = mytalk_ai(&engine, soldier_id);
        assert!(ai.speech_in_flight);
        assert_eq!(ai.current_remark, Remark::Arrow);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    engine.control.frame_counter = 103;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 103);
    engine.process_npc_speech(&assets);
    let ai = mytalk_ai(&engine, soldier_id);
    assert!(!ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        vec![StimulusType::EventMyTalk1]
    );
}

#[test]
fn missing_exclamation_duration_completes_mytalk_at_next_boundary() {
    use crate::ai::{Remark, StimulusType};

    let (mut engine, soldier_id, assets) = build_mytalk_timing_test(None);
    engine.process_npc_speech(&assets);

    assert_eq!(engine.feedback.sound_sim.playing_exclamations.len(), 1);
    assert_eq!(
        engine.feedback.sound_sim.playing_exclamations[0].finish_frame, 100,
        "missing metadata must not fabricate a 75-frame speech"
    );
    let ai = mytalk_ai(&engine, soldier_id);
    assert!(ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::Arrow);

    engine.control.frame_counter = 101;
    super::tick::drain_matured_exclamations(&mut engine.feedback.sound_sim, 101);
    engine.process_npc_speech(&assets);

    let ai = mytalk_ai(&engine, soldier_id);
    assert_eq!(engine.control.frame_counter, 101);
    assert!(!ai.speech_in_flight);
    assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
    assert_eq!(ai.outbox.speech.mytalk_flags, 0);
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        vec![StimulusType::EventMyTalk1]
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
