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
        let value = AiController {
            old_state: (7u32 + seed) as i32,
            blood_alcohol: (12u32 + seed) as u8,
            initial_action: (13u32 + seed),
            number_of_looks: (14u32 + seed) as u8,
            has_patrol_path: seed & (1 << 2) != 0,
            can_move: seed & (1 << 1) != 0,
            stop_before_end_of_path: seed & (1 << 2) != 0,
            use_max_norm_to_stop_before_end_of_path: seed & (1 << 3) != 0,
            stop_before_end_of_path_distance: (21u32 + seed) as u16,
            think_recursion_depth: (22u32 + seed) as u8,
            macro_command: vec![(26u32 + seed) as u8, seed as u8],
            macro_command_offset: (27u32 + seed) as usize,
            number_of_remaining_macro_bytes: (29u32 + seed) as u16,
            macro_in_progress: seed & (1 << 1) != 0,
            macro_started_in_this_frame: seed & (1 << 2) != 0,
            timer_is_running: seed & (1 << 1) != 0,
            when_does_timer_ring: (39u32 + seed),
            macro_timer_is_running: seed & (1 << 3) != 0,
            when_does_macro_timer_ring: (41u32 + seed),
            standing_around_timer: (42u32 + seed) as u16,
            sorrow_level: (43u32 + seed) as u16,
            is_master: seed & (1 << 1) != 0,
            first_try: seed & (1 << 1) != 0,
            panic_center_x: (51u32 + seed) as f32 / 7.0,
            panic_center_y: (52u32 + seed) as f32 / 7.0,
            lasting_panic_runs: (53u32 + seed) as u8,
            directed_panic: seed & (1 << 1) != 0,
            couldnt_reachpoint: seed & (1 << 1) != 0,
            already_on_point: seed & (1 << 2) != 0,
            already_turned: seed & (1 << 3) != 0,
            completion_latch_inside_think: seed & (1 << 0) != 0,
            likes_to_sit_around: seed & (1 << 1) != 0,
            special_action: seed & (1 << 2) != 0,
            remaining_tequila_gulps: (64u32 + seed) as u8,
            friends_are_alerted: seed & (1 << 0) != 0,
            is_stay_at_home: seed & (1 << 1) != 0,
            was_busy: seed & (1 << 3) != 0,
            script_locked: seed & (1 << 1) != 0,
            remember_events: seed & (1 << 2) != 0,
            leave_house_number: (72u32 + seed) as u16,
            last_hint_actuality: (73u32 + seed),
            looking_for_help_because_enemy_seen: seed & (1 << 3) != 0,
            synchronize_index: (81u32 + seed) as u16,
            delta_sorrow_level: (82u32 + seed) as u16,
            frame_when_enemy_detected: (84u32 + seed),
            inside_halt_method: seed & (1 << 0) != 0,
            forbidden_remark_ids: vec![(88u32 + seed), seed],
            current_remark_flags: (91u32 + seed) as u16,
            next_macro_rand: (92u32 + seed) as u8,
            next_macro_rand_forecasted: seed & (1 << 0) != 0,
            emoticon_expiration_date: (95u32 + seed),
            emoticon_has_expiration_date: seed & (1 << 3) != 0,
            knocked_out_in_money_fight: seed & (1 << 1) != 0,
            looted_after_money_fight: seed & (1 << 2) != 0,
            patrol_stopped: seed & (1 << 3) != 0,
            patrol_direction: (105u32 + seed) as u16,
            needs_patrol_reinit: seed & (1 << 1) != 0,
            got_the_beggar_trick: seed & (1 << 2) != 0,
            debug_view_cone_enabled: seed & (1 << 0) != 0,
            stuck_counter: (112u32 + seed) as u16,
            has_script_filter_override: seed & (1 << 1) != 0,
            initial_view_direction: (117u32 + seed) as u16,
            max_visibility: (118u32 + seed),
            cached_frame: (119u32 + seed),
            cached_in_building: seed & (1 << 3) != 0,
            ..Default::default()
        };
        assert_projection_matches_wire!(value, PersistedAiController, AiController);
    }
}

#[test]
fn ai_global_state_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let value = AiGlobalState {
            green_alert_soldiers: (1u32 + seed) as u16,
            yellow_alert_soldiers: (2u32 + seed) as u16,
            red_alert_soldiers: (3u32 + seed) as u16,
            stupid_soldiers_cheat: seed & (1 << 2) != 0,
            freeze: seed & (1 << 3) != 0,
            saved_random_seed: (14u32 + seed) as i64,
            remarks_forbidden_till_frame: vec![(15u32 + seed), seed],
            attribute_display: seed & (1 << 1) != 0,
            speech_display: seed & (1 << 2) != 0,
            golden_eye_mode: seed & (1 << 3) != 0,
            ezekiel_2517: seed & (1 << 0) != 0,
            current_speech_variant: (22u32 + seed) as u16,
            next_repulsive_point_id: (24u32 + seed) as i32,
            ..Default::default()
        };
        assert_projection_matches_wire!(value, PersistedAiGlobalState, AiGlobalState);
    }
}

#[test]
fn enemy_ai_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let value = EnemyAi {
            pending_special_strike: seed & (1 << 1) != 0,
            pending_sword_strike_consideration: seed & (1 << 2) != 0,
            pending_combat_insult_after_strike_consideration: seed & (1 << 3) != 0,
            pc_missed: seed & (1 << 1) != 0,
            pc_gone_away_in_this_direction: (7u32 + seed) as u16,
            frame_when_missed_charly: (8u32 + seed),
            investigating_distraction: seed & (1 << 2) != 0,
            last_seek_direction_index: (12u32 + seed) as u8,
            beggar_is_npc: seed & (1 << 1) != 0,
            current_task_priority: (15u32 + seed) as u16,
            minimal_task_priority: (16u32 + seed) as u16,
            new_task_priority: (17u32 + seed) as u16,
            number_of_different_checkpoints: (18u32 + seed) as u8,
            thirsty: seed & (1 << 2) != 0,
            position_change_locked_for_test: seed & (1 << 3) != 0,
            seen_dead_body: seed & (1 << 3) != 0,
            seeking_charly: seed & (1 << 0) != 0,
            my_seek_points: vec![(26u32 + seed) as u16, seed as u16],
            seek_point_view_directions: vec![(31u32 + seed) as u16, seed as u16],
            old_odds: (33u32 + seed) as i16,
            gather_direction: (35u32 + seed) as u16,
            gather_position_instructed: seed & (1 << 3) != 0,
            previous_state: (39u32 + seed) as i32,
            previous_substate: (40u32 + seed) as i32,
            reported_to_officer: seed & (1 << 0) != 0,
            missed_soldier_timer: (42u32 + seed) as u16,
            old_money: (43u32 + seed) as u16,
            shield_bearer_direction: (50u32 + seed) as u16,
            phalanx_aborted: seed & (1 << 2) != 0,
            changed_to_alert_path: seed & (1 << 3) != 0,
            pending_group_instruction_seek_flags: (57u32 + seed) as u16,
            pending_group_instruction_clear_location_after_accept: seed & (1 << 1) != 0,
            my_archery_sector_index: (61u32 + seed) as u16,
            my_archery_point_increment: (63u32 + seed) as i8,
            enemy_seen_below: seed & (1 << 3) != 0,
            enemy_had_this_elevation: (65u32 + seed) as u16,
            fleeing_seen_enemy_counter: (70u32 + seed) as u16,
            character_id: (72u32 + seed),
            old_life_points: (73u32 + seed) as u8,
            initial_life_points: (74u32 + seed) as u8,
            ambush_point_array_reset: seed & (1 << 3) != 0,
            reset_battle_decision: seed & (1 << 2) != 0,
            soldier_profile_iq: (80u32 + seed) as u16,
            soldier_profile_courage: (81u32 + seed) as u16,
            soldier_profile_shooting: (82u32 + seed) as u16,
            soldier_profile_vip: seed & (1 << 2) != 0,
            soldier_profile_bee_time: (84u32 + seed) as u16,
            soldier_profile_pride: (85u32 + seed) as u16,
            soldier_profile_hearing_factor: (86u32 + seed) as f32 / 7.0,
            soldier_profile_initiative: (88u32 + seed) as u16,
            soldier_profile_beer: (89u32 + seed) as u16,
            ale_reliable_distraction: seed & (1 << 1) != 0,
            soldier_profile_money: (91u32 + seed) as u16,
            soldier_profile_apple: (92u32 + seed) as u16,
            soldier_profile_whistle: (93u32 + seed) as u16,
            soldier_profile_duty: seed & (1 << 1) != 0,
            soldier_profile_endurance: (95u32 + seed) as u16,
            is_vip: seed & (1 << 3) != 0,
            sword_range: (97u32 + seed) as u16,
            hth_weapon_id: (98u32 + seed),
            sword_is_charge_weapon: seed & (1 << 2) != 0,
            next_sword_strike_frame: (100u32 + seed),
            company_number: (101u32 + seed) as u16,
            attentive: seed & (1 << 3) != 0,
            will_be_attentive: seed & (1 << 0) != 0,
            forced_attentive: seed & (1 << 1) != 0,
            tower_guard: seed & (1 << 0) != 0,
            combat_trainer: seed & (1 << 1) != 0,
            is_archer_unit: seed & (1 << 2) != 0,
            ..Default::default()
        };
        assert_projection_matches_wire!(value, PersistedEnemyAi, EnemyAi);
    }
}

#[test]
fn friendly_ai_scalar_projection_matrix() {
    for seed in 0..16u32 {
        let value = FriendlyAi {
            beggar_dont_talk_counter: (2u32 + seed) as u16,
            fleeing_seen_enemy_counter: (3u32 + seed) as u16,
            wants_to_talk: seed & (1 << 3) != 0,
            can_go_away: seed & (1 << 1) != 0,
            ..Default::default()
        };
        assert_projection_matches_wire!(value, PersistedFriendlyAi, FriendlyAi);
    }
}

fn populated_controller() -> AiController {
    AiController {
        me: 42,
        old_state: i32::MIN + 7,
        primary_target: Some(AiEntityHandle::new(0)),
        macro_command: vec![0, 1, 254, 255],
        macro_command_offset: 3,
        think_recursion_depth: 8,
        open_end_think_frames: 7,
        engine_deferred_end_think_frames: 5,
        engine_completion_verdict_resolved: true,
        forbidden_remark_ids: vec![9, 3, 9],
        list_us: vec![17, 0, 8],
        stimulus_queue: vec![provenance_stimulus(SelfStimulusOrigin::Condolation)],
        outbox: populated_outbox(),
        ..Default::default()
    }
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
    let mut raw = AiGlobalState {
        saved_random_seed: i64::MIN + 31,
        green_alert_soldiers: 17,
        freeze: true,
        all_soldier_handles: std::sync::Arc::new(vec![19, 0, 7]),
        ..Default::default()
    };
    raw.primary_target_multiplicity_scratch.insert(7, 19);
    raw.primary_target_multiplicity_initialized = true;
    let raw_clone = raw.clone();
    let restored =
        assert_projection_matches_wire!(raw.clone(), PersistedAiGlobalState, AiGlobalState);
    // Target selection now persists only its live actor state, not a dead
    // global compensation ledger from the former batched AI scheduler.
    assert!(
        serde_json::to_value(&restored)
            .unwrap()
            .get("same_frame_target_claims")
            .is_none()
    );
    assert!(restored.primary_target_multiplicity_scratch.is_empty());
    assert!(!restored.primary_target_multiplicity_initialized);
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
    let enemy = EnemyAi {
        base: populated_controller(),
        previous_state: i32::MIN,
        previous_substate: i32::MAX,
        missed_pc: Some(AiEntityHandle::new(0)),
        pending_group_instruction_candidates: vec![
            (3, Position::default()),
            (1, Position::default()),
        ],
        last_stimulus_dispatched_to_patrol: Some(provenance_stimulus(
            SelfStimulusOrigin::Condolation,
        )),
        ..Default::default()
    };
    let restored = assert_projection_matches_wire!(enemy, PersistedEnemyAi, EnemyAi);
    assert_eq!(
        restored
            .last_stimulus_dispatched_to_patrol
            .unwrap()
            .self_origin,
        SelfStimulusOrigin::Ordinary
    );
    let friendly = FriendlyAi {
        base: populated_controller(),
        last_talk_partner: Some(AiEntityHandle::new(0)),
        can_go_away: true,
        ..Default::default()
    };
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
