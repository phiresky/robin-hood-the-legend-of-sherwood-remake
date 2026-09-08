use super::*;
use robin_util::state_hash::compute;
mod legacy;
use legacy::LegacyWire;

// The test-only legacy declarations are independent of the projections.
// Check field order, optional-handle tags, defaults, native layout and hash
// semantics without putting a serializer on the capture path.
macro_rules! assert_projection_matches_wire {
    ($runtime:expr, $persisted:ty, $live:ty) => {{
        let runtime = $runtime;
        assert_eq!(compute(&runtime), runtime.legacy_hash());
        assert_eq!(bitcode::encode(&runtime), runtime.legacy_native_bytes());
        let json = runtime.legacy_json();
        assert_eq!(serde_json::to_string(&runtime).unwrap(), json);
        let persisted = <$persisted>::capture(&runtime);
        assert_eq!(serde_json::to_string(&persisted).unwrap(), json);
        let restored = persisted.into_runtime();
        let legacy = <$live as LegacyWire>::legacy_from_json(&json);
        assert_eq!(serde_json::to_string(&restored).unwrap(), json);
        assert_eq!(bitcode::encode(&restored), bitcode::encode(&legacy));
        assert_eq!(bitcode::encode(&restored), bitcode::encode(&runtime));
        assert_eq!(compute(&restored), compute(&legacy));
        let dto: $persisted = serde_json::from_str(&json).unwrap();
        assert_eq!(
            bitcode::encode(&dto.into_runtime()),
            bitcode::encode(&legacy)
        );
        restored
    }};
}

#[test]
fn ai_controller_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let mut value = AiController::default();
        value.old_state = (7u32 + seed) as i32;
        value.blood_alcohol = (12u32 + seed) as u8;
        value.initial_action = (13u32 + seed) as u32;
        value.number_of_looks = (14u32 + seed) as u8;
        value.has_patrol_path = seed & (1 << 2) != 0;
        value.can_move = seed & (1 << 1) != 0;
        value.stop_before_end_of_path = seed & (1 << 2) != 0;
        value.use_max_norm_to_stop_before_end_of_path = seed & (1 << 3) != 0;
        value.stop_before_end_of_path_distance = (21u32 + seed) as u16;
        value.think_recursion_depth = (22u32 + seed) as u8;
        value.macro_command = vec![(26u32 + seed) as u8, seed as u8];
        value.macro_command_offset = (27u32 + seed) as usize;
        value.number_of_remaining_macro_bytes = (29u32 + seed) as u16;
        value.macro_in_progress = seed & (1 << 1) != 0;
        value.macro_started_in_this_frame = seed & (1 << 2) != 0;
        value.timer_is_running = seed & (1 << 1) != 0;
        value.when_does_timer_ring = (39u32 + seed) as u32;
        value.macro_timer_is_running = seed & (1 << 3) != 0;
        value.when_does_macro_timer_ring = (41u32 + seed) as u32;
        value.standing_around_timer = (42u32 + seed) as u16;
        value.sorrow_level = (43u32 + seed) as u16;
        value.is_master = seed & (1 << 1) != 0;
        value.first_try = seed & (1 << 1) != 0;
        value.panic_center_x = (51u32 + seed) as f32 / 7.0;
        value.panic_center_y = (52u32 + seed) as f32 / 7.0;
        value.lasting_panic_runs = (53u32 + seed) as u8;
        value.directed_panic = seed & (1 << 1) != 0;
        value.couldnt_reachpoint = seed & (1 << 1) != 0;
        value.already_on_point = seed & (1 << 2) != 0;
        value.already_turned = seed & (1 << 3) != 0;
        value.completion_latch_inside_think = seed & (1 << 0) != 0;
        value.likes_to_sit_around = seed & (1 << 1) != 0;
        value.special_action = seed & (1 << 2) != 0;
        value.remaining_tequila_gulps = (64u32 + seed) as u8;
        value.friends_are_alerted = seed & (1 << 0) != 0;
        value.is_stay_at_home = seed & (1 << 1) != 0;
        value.was_busy = seed & (1 << 3) != 0;
        value.script_locked = seed & (1 << 1) != 0;
        value.remember_events = seed & (1 << 2) != 0;
        value.leave_house_number = (72u32 + seed) as u16;
        value.last_hint_actuality = (73u32 + seed) as u32;
        value.looking_for_help_because_enemy_seen = seed & (1 << 3) != 0;
        value.synchronize_index = (81u32 + seed) as u16;
        value.delta_sorrow_level = (82u32 + seed) as u16;
        value.frame_when_enemy_detected = (84u32 + seed) as u32;
        value.inside_halt_method = seed & (1 << 0) != 0;
        value.forbidden_remark_ids = vec![(88u32 + seed) as u32, seed as u32];
        value.current_remark_flags = (91u32 + seed) as u16;
        value.next_macro_rand = (92u32 + seed) as u8;
        value.next_macro_rand_forecasted = seed & (1 << 0) != 0;
        value.emoticon_expiration_date = (95u32 + seed) as u32;
        value.emoticon_has_expiration_date = seed & (1 << 3) != 0;
        value.knocked_out_in_money_fight = seed & (1 << 1) != 0;
        value.looted_after_money_fight = seed & (1 << 2) != 0;
        value.patrol_stopped = seed & (1 << 3) != 0;
        value.patrol_direction = (105u32 + seed) as u16;
        value.needs_patrol_reinit = seed & (1 << 1) != 0;
        value.got_the_beggar_trick = seed & (1 << 2) != 0;
        value.debug_view_cone_enabled = seed & (1 << 0) != 0;
        value.stuck_counter = (112u32 + seed) as u16;
        value.has_script_filter_override = seed & (1 << 1) != 0;
        value.initial_view_direction = (117u32 + seed) as u16;
        value.max_visibility = (118u32 + seed) as u32;
        value.cached_frame = (119u32 + seed) as u32;
        value.cached_in_building = seed & (1 << 3) != 0;
        assert_projection_matches_wire!(value, PersistedAiController, AiController);
    }
}

#[test]
fn ai_global_state_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let mut value = AiGlobalState::default();
        value.green_alert_soldiers = (1u32 + seed) as u16;
        value.yellow_alert_soldiers = (2u32 + seed) as u16;
        value.red_alert_soldiers = (3u32 + seed) as u16;
        value.there_are_royalist_soldiers = seed & (1 << 3) != 0;
        value.there_are_lacklandist_soldiers = seed & (1 << 0) != 0;
        value.stupid_soldiers_cheat = seed & (1 << 2) != 0;
        value.freeze = seed & (1 << 3) != 0;
        value.saved_random_seed = (14u32 + seed) as i64;
        value.remarks_forbidden_till_frame = vec![(15u32 + seed) as u32, seed as u32];
        value.attribute_display = seed & (1 << 1) != 0;
        value.speech_display = seed & (1 << 2) != 0;
        value.golden_eye_mode = seed & (1 << 3) != 0;
        value.ezekiel_2517 = seed & (1 << 0) != 0;
        value.current_speech_variant = (22u32 + seed) as u16;
        value.next_repulsive_point_id = (24u32 + seed) as i32;
        assert_projection_matches_wire!(value, PersistedAiGlobalState, AiGlobalState);
    }
}

#[test]
fn enemy_ai_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let mut value = EnemyAi::default();
        value.pending_special_strike = seed & (1 << 1) != 0;
        value.pending_sword_strike_consideration = seed & (1 << 2) != 0;
        value.pending_combat_insult_after_strike_consideration = seed & (1 << 3) != 0;
        value.pc_missed = seed & (1 << 1) != 0;
        value.pc_gone_away_in_this_direction = (7u32 + seed) as u16;
        value.frame_when_missed_charly = (8u32 + seed) as u32;
        value.investigating_distraction = seed & (1 << 2) != 0;
        value.last_seek_direction_index = (12u32 + seed) as u8;
        value.beggar_is_npc = seed & (1 << 1) != 0;
        value.current_task_priority = (15u32 + seed) as u16;
        value.minimal_task_priority = (16u32 + seed) as u16;
        value.new_task_priority = (17u32 + seed) as u16;
        value.number_of_different_checkpoints = (18u32 + seed) as u8;
        value.thirsty = seed & (1 << 2) != 0;
        value.position_change_locked_for_test = seed & (1 << 3) != 0;
        value.seen_dead_body = seed & (1 << 3) != 0;
        value.seeking_charly = seed & (1 << 0) != 0;
        value.my_seek_points = vec![(26u32 + seed) as u16, seed as u16];
        value.seek_point_view_directions = vec![(31u32 + seed) as u16, seed as u16];
        value.old_odds = (33u32 + seed) as i16;
        value.gather_direction = (35u32 + seed) as u16;
        value.gather_position_instructed = seed & (1 << 3) != 0;
        value.previous_state = (39u32 + seed) as i32;
        value.previous_substate = (40u32 + seed) as i32;
        value.reported_to_officer = seed & (1 << 0) != 0;
        value.missed_soldier_timer = (42u32 + seed) as u16;
        value.old_money = (43u32 + seed) as u16;
        value.shield_bearer_direction = (50u32 + seed) as u16;
        value.phalanx_aborted = seed & (1 << 2) != 0;
        value.changed_to_alert_path = seed & (1 << 3) != 0;
        value.pending_group_instruction_seek_flags = (57u32 + seed) as u16;
        value.pending_group_instruction_clear_location_after_accept = seed & (1 << 1) != 0;
        value.my_archery_sector_index = (61u32 + seed) as u16;
        value.my_archery_point_increment = (63u32 + seed) as i8;
        value.enemy_seen_below = seed & (1 << 3) != 0;
        value.enemy_had_this_elevation = (65u32 + seed) as u16;
        value.fleeing_seen_enemy_counter = (70u32 + seed) as u16;
        value.character_id = (72u32 + seed) as u32;
        value.old_life_points = (73u32 + seed) as u8;
        value.initial_life_points = (74u32 + seed) as u8;
        value.ambush_point_array_reset = seed & (1 << 3) != 0;
        value.reset_battle_decision = seed & (1 << 2) != 0;
        value.soldier_profile_iq = (80u32 + seed) as u16;
        value.soldier_profile_courage = (81u32 + seed) as u16;
        value.soldier_profile_shooting = (82u32 + seed) as u16;
        value.soldier_profile_vip = seed & (1 << 2) != 0;
        value.soldier_profile_bee_time = (84u32 + seed) as u16;
        value.soldier_profile_pride = (85u32 + seed) as u16;
        value.soldier_profile_hearing_factor = (86u32 + seed) as f32 / 7.0;
        value.soldier_profile_initiative = (88u32 + seed) as u16;
        value.soldier_profile_beer = (89u32 + seed) as u16;
        value.ale_reliable_distraction = seed & (1 << 1) != 0;
        value.soldier_profile_money = (91u32 + seed) as u16;
        value.soldier_profile_apple = (92u32 + seed) as u16;
        value.soldier_profile_whistle = (93u32 + seed) as u16;
        value.soldier_profile_duty = seed & (1 << 1) != 0;
        value.soldier_profile_endurance = (95u32 + seed) as u16;
        value.is_vip = seed & (1 << 3) != 0;
        value.sword_range = (97u32 + seed) as u16;
        value.hth_weapon_id = (98u32 + seed) as u32;
        value.sword_is_charge_weapon = seed & (1 << 2) != 0;
        value.next_sword_strike_frame = (100u32 + seed) as u32;
        value.company_number = (101u32 + seed) as u16;
        value.attentive = seed & (1 << 3) != 0;
        value.will_be_attentive = seed & (1 << 0) != 0;
        value.forced_attentive = seed & (1 << 1) != 0;
        value.tower_guard = seed & (1 << 0) != 0;
        value.combat_trainer = seed & (1 << 1) != 0;
        value.is_archer_unit = seed & (1 << 2) != 0;
        assert_projection_matches_wire!(value, PersistedEnemyAi, EnemyAi);
    }
}

#[test]
fn friendly_ai_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let mut value = FriendlyAi::default();
        value.beggar_dont_talk_counter = (2u32 + seed) as u16;
        value.fleeing_seen_enemy_counter = (3u32 + seed) as u16;
        value.wants_to_talk = seed & (1 << 3) != 0;
        value.can_go_away = seed & (1 << 1) != 0;
        assert_projection_matches_wire!(value, PersistedFriendlyAi, FriendlyAi);
    }
}

fn populated_controller() -> AiController {
    let mut value = AiController::default();
    value.me = 42;
    value.old_state = i32::MIN + 7;
    value.primary_target = Some(AiEntityHandle::new(0));
    value.macro_command = vec![0, 1, 254, 255];
    value.macro_command_offset = 3;
    value.think_recursion_depth = 8;
    value.open_end_think_frames = 7;
    value.engine_deferred_end_think_frames = 5;
    value.engine_completion_verdict_resolved = true;
    value.forbidden_remark_ids = vec![9, 3, 9];
    value.list_us = vec![17, 0, 8];
    value.stimulus_queue = vec![provenance_stimulus(SelfStimulusOrigin::Condolation)];
    value.outbox = populated_outbox();
    value
}

fn provenance_stimulus(origin: SelfStimulusOrigin) -> Stimulus {
    let mut value = Stimulus::new(StimulusType::EventReachPoint);
    value.info = StimulusInfo::LegacyInvalidType(i32::MIN + 9);
    value.owner = Some(AiEntityHandle::new(0));
    value.to_whole_patrol = true;
    value.self_origin = origin;
    value
}

fn populated_outbox() -> AiOutbox {
    let mut value = AiOutbox::default();
    value.patrol.direction_broadcast = Some(65535);
    value.detection.stimuli = vec![provenance_stimulus(SelfStimulusOrigin::EngineCompletion)];
    value.detection.mark_alerted = true;
    value.reentrant.engine_drains_after_script_go_on = true;
    value.reentrant.self_stimuli = vec![
        QueuedSelfStimulus::new(StimulusType::EventDone, SelfStimulusOrigin::Condolation),
        QueuedSelfStimulus::new(
            StimulusType::EventReachPoint,
            SelfStimulusOrigin::EngineCompletion,
        ),
    ];
    value.reentrant.finish_macro_after_self_stimuli = true;
    value.reentrant.battle_observe_completion_pending = true;
    value.reentrant.waypoint_script_reach_point = Some((PathId::new(7).unwrap(), 2));
    value.reentrant.owner_work = vec![
        AiOwnerWork::NearbyCiviliansPanic180,
        AiOwnerWork::ResumeSoldierGiveReportAfterSpeech { current_frame: 198 },
        AiOwnerWork::NearbyCiviliansPanic,
    ];
    value.actor.orders = Vec::new();
    value.actor.set_direction = Some(-127);
    value.actor.focus = Some(AiEntityHandle::new(0));
    value.actor.additional_halts = 3;
    value.music.instant_change = true;
    value.recovery.inform_resurrection = true;
    value
}

#[test]
fn controller_projection_matches_existing_json_native_and_hash_contracts() {
    let raw = populated_controller();
    let raw_clone = raw.clone();
    let restored =
        assert_projection_matches_wire!(raw.clone(), PersistedAiController, AiController);
    assert_eq!(raw_clone.open_end_think_frames, 7);
    assert_eq!(raw_clone.engine_deferred_end_think_frames, 5);
    assert!(raw_clone.engine_completion_verdict_resolved);
    assert_eq!(
        raw_clone.stimulus_queue[0].self_origin,
        SelfStimulusOrigin::Condolation
    );
    assert_eq!(restored.open_end_think_frames, 0);
    assert_eq!(restored.engine_deferred_end_think_frames, 0);
    assert!(!restored.engine_completion_verdict_resolved);
    assert_eq!(restored.think_recursion_depth, 8);
    assert_eq!(
        restored.stimulus_queue[0].self_origin,
        SelfStimulusOrigin::Ordinary
    );
    assert_eq!(compute(&raw), compute(&restored));
}

#[test]
fn global_projection_reconstructs_nonpersisted_scratch_without_changing_hash() {
    let mut raw = AiGlobalState::default();
    raw.saved_random_seed = i64::MIN + 31;
    raw.green_alert_soldiers = 17;
    raw.freeze = true;
    raw.all_soldier_handles = std::sync::Arc::new(vec![19, 0, 7]);
    raw.same_frame_target_claims = vec![(19, 7), (0, 7)];
    raw.primary_target_multiplicity_scratch.insert(7, 19);
    raw.primary_target_multiplicity_initialized = true;
    let raw_clone = raw.clone();
    let restored =
        assert_projection_matches_wire!(raw.clone(), PersistedAiGlobalState, AiGlobalState);
    assert!(restored.primary_target_multiplicity_scratch.is_empty());
    assert!(!restored.primary_target_multiplicity_initialized);
    assert_eq!(restored.same_frame_target_claims, vec![(19, 7), (0, 7)]);
    assert_eq!(
        raw_clone.primary_target_multiplicity_scratch.get(&7),
        Some(&19)
    );
    assert!(raw_clone.primary_target_multiplicity_initialized);
    // Historical StateHash inferred this opt-out from serde(skip). The live
    // declaration now states it explicitly because serde delegates to the DTO.
    assert_eq!(compute(&raw), compute(&restored));
}

#[test]
fn outbox_projection_preserves_fifo_and_only_reconstructs_runtime_provenance() {
    let raw = populated_outbox();
    let raw_clone = raw.clone();
    let restored = assert_projection_matches_wire!(raw, PersistedAiOutbox, AiOutbox);
    assert!(raw_clone.reentrant.engine_drains_after_script_go_on);
    assert_eq!(
        raw_clone.reentrant.self_stimuli[1].origin,
        SelfStimulusOrigin::EngineCompletion
    );
    assert!(!restored.reentrant.engine_drains_after_script_go_on);
    assert!(restored.reentrant.finish_macro_after_self_stimuli);
    assert!(restored.reentrant.battle_observe_completion_pending);
    assert_eq!(
        restored.reentrant.self_stimuli[0].origin,
        SelfStimulusOrigin::Ordinary
    );
    assert_eq!(
        restored.detection.stimuli[0].self_origin,
        SelfStimulusOrigin::Ordinary
    );
    assert!(matches!(
        restored.reentrant.owner_work[0],
        AiOwnerWork::NearbyCiviliansPanic180
    ));
    assert!(matches!(
        restored.reentrant.owner_work[1],
        AiOwnerWork::ResumeSoldierGiveReportAfterSpeech { current_frame: 198 }
    ));
}

#[test]
fn enemy_and_friendly_projection_recurse_into_base_and_last_patrol_stimulus() {
    let mut enemy = EnemyAi::default();
    enemy.base = populated_controller();
    enemy.previous_state = i32::MIN;
    enemy.previous_substate = i32::MAX;
    enemy.missed_pc = Some(AiEntityHandle::new(0));
    enemy.pending_group_instruction_candidates =
        vec![(3, Position::default()), (1, Position::default())];
    enemy.last_stimulus_dispatched_to_patrol =
        Some(provenance_stimulus(SelfStimulusOrigin::Condolation));
    let restored = assert_projection_matches_wire!(enemy, PersistedEnemyAi, EnemyAi);
    assert_eq!(
        restored
            .last_stimulus_dispatched_to_patrol
            .unwrap()
            .self_origin,
        SelfStimulusOrigin::Ordinary
    );
    let mut friendly = FriendlyAi::default();
    friendly.base = populated_controller();
    friendly.last_talk_partner = Some(AiEntityHandle::new(0));
    friendly.can_go_away = true;
    let restored = assert_projection_matches_wire!(friendly, PersistedFriendlyAi, FriendlyAi);
    assert_eq!(restored.base.open_end_think_frames, 0);
    assert_eq!(restored.last_talk_partner, Some(AiEntityHandle::new(0)));
}

#[test]
fn stimulus_projection_preserves_transparent_queue_and_tagged_owner_wire() {
    let raw = QueuedSelfStimulus::new(
        StimulusType::EventDone,
        SelfStimulusOrigin::EngineCompletion,
    );
    let restored =
        assert_projection_matches_wire!(raw, PersistedQueuedSelfStimulus, QueuedSelfStimulus);
    assert_eq!(
        serde_json::to_string(&PersistedQueuedSelfStimulus::capture(&raw)).unwrap(),
        "\"EventDone\""
    );
    assert_eq!(restored.origin, SelfStimulusOrigin::Ordinary);
    let raw = provenance_stimulus(SelfStimulusOrigin::EngineCompletion);
    let restored = assert_projection_matches_wire!(raw, PersistedStimulus, Stimulus);
    assert_eq!(restored.owner, Some(AiEntityHandle::new(0)));
    assert!(
        serde_json::to_string(&restored)
            .unwrap()
            .contains("\"owner\":{\"entity\":0}")
    );
}
