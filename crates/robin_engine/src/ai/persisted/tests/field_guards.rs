// Exhaustive field classification for the persisted AI owners.
//
// Each destructure names every field with no `..`, so adding a field to one
// of these owners fails to compile until it is listed here as `persisted`
// (plain derive) or `skipped` (`#[serde(skip)]`, which must also be reset in
// `PersistedProjection::clear_runtime_only_state` in `ai/persisted.rs`).
use super::*;

macro_rules! classify_fields {
    ($value:expr, $ty:ident {
        persisted: [$($persisted:ident),* $(,)?],
        skipped: [$($skipped:ident),* $(,)?] $(,)?
    }) => {{
        let $ty {
            $($persisted: _,)*
            $($skipped: _,)*
        } = $value;
    }};
}

fn classify_ai_controller(value: &AiController) {
    classify_fields!(
        value,
        AiController {
            persisted: [
                me,
                owner_entity_id,
                path_id,
                alert_path_id,
                current_state,
                current_substate,
                old_state,
                current_music_alert_status,
                view_alert_status,
                substate_at_last_timer_launch,
                attitude,
                blood_alcohol,
                initial_action,
                number_of_looks,
                has_patrol_path,
                patrol_path,
                detached_patrol_path_status,
                can_move,
                stop_before_end_of_path,
                use_max_norm_to_stop_before_end_of_path,
                stop_before_end_of_path_distance,
                think_recursion_depth,
                macro_command,
                macro_command_offset,
                macro_command_waypoint,
                number_of_remaining_macro_bytes,
                macro_in_progress,
                macro_started_in_this_frame,
                primary_target,
                friend_in_trouble,
                detected_body,
                interesting_object,
                antagonist,
                last_stimulus_actor,
                timer_is_running,
                when_does_timer_ring,
                macro_timer_is_running,
                when_does_macro_timer_ring,
                standing_around_timer,
                sorrow_level,
                last_stimulus,
                last_stimulus_multiplicity,
                is_master,
                master,
                seek_position,
                alert_soldiers_point,
                first_try,
                panic_center_x,
                panic_center_y,
                lasting_panic_runs,
                directed_panic,
                list_us,
                list_alerted_us,
                list_staying_us,
                couldnt_reachpoint,
                already_on_point,
                already_turned,
                completion_latch_inside_think,
                likes_to_sit_around,
                special_action,
                remaining_tequila_gulps,
                friends_are_alerted,
                is_stay_at_home,
                locks_flag_field,
                was_busy,
                // Persisted; nested `Stimulus::self_origin` is skipped.
                stimulus_queue,
                script_locked,
                remember_events,
                leave_house_number,
                last_hint_actuality,
                last_hint_subject,
                my_door_index,
                looking_for_help_because_enemy_seen,
                forgotten_objects,
                object_of_desire,
                checkpoint_charly,
                synchronize_charly,
                synchronize_index,
                delta_sorrow_level,
                missed_in_action,
                frame_when_enemy_detected,
                inside_halt_method,
                synchronizing_actors,
                default_path_walking_flags,
                forbidden_remark_ids,
                initial_view_cone,
                current_remark,
                current_remark_flags,
                next_macro_rand,
                next_macro_rand_forecasted,
                current_emoticon_type,
                emoticon_expiration_date,
                emoticon_has_expiration_date,
                my_reconnaissance_report,
                knocked_out_in_money_fight,
                looted_after_money_fight,
                patrol_chief,
                patrol,
                missed_patrol_members,
                theoretical_patrol,
                patrol_stopped,
                patrol_direction,
                needs_patrol_reinit,
                got_the_beggar_trick,
                ai_log,
                debug_view_cone_enabled,
                last_goto_destination,
                last_goto_flags,
                stuck_counter,
                // Persisted; nested outbox scratch is skipped.
                outbox,
                has_script_filter_override,
                last_synced_focus_target,
                initial_position,
                initial_view_direction,
                max_visibility,
                cached_frame,
                cached_in_building,
            ],
            skipped: [
                open_end_think_frames,
                engine_deferred_end_think_frames,
                engine_completion_verdict_resolved,
            ],
        }
    );
}

fn classify_ai_global_state(value: &AiGlobalState) {
    classify_fields!(
        value,
        AiGlobalState {
            persisted: [
                green_alert_soldiers,
                yellow_alert_soldiers,
                red_alert_soldiers,
                soldier_camps,
                stupid_soldiers_cheat,
                freeze,
                overall_alert_status,
                overall_villain_alert_status,
                ambush_points,
                seek_points,
                archery_sectors,
                saved_random_seed,
                remarks_forbidden_till_frame,
                forbidden_remarks,
                screen_remarks,
                attribute_display,
                speech_display,
                golden_eye_mode,
                ezekiel_2517,
                current_speech_variant,
                repulsive_points,
                next_repulsive_point_id,
                door_seek_infos,
                reinforcement_doors,
                houses,
                door_rally_points,
                all_soldier_handles,
            ],
            skipped: [
                primary_target_multiplicity_scratch,
                primary_target_multiplicity_initialized,
            ],
        }
    );
}

fn classify_queued_self_stimulus(value: &QueuedSelfStimulus) {
    classify_fields!(
        value,
        QueuedSelfStimulus {
            // `#[serde(transparent)]`: serialized as the bare stimulus name.
            persisted: [stimulus_type],
            skipped: [origin],
        }
    );
}

fn classify_stimulus(value: &Stimulus) {
    classify_fields!(
        value,
        Stimulus {
            persisted: [stimulus_type, info, owner, to_whole_patrol],
            skipped: [self_origin],
        }
    );
}

fn classify_ai_outbox(value: &AiOutbox) {
    classify_fields!(
        value,
        AiOutbox {
            persisted: [patrol, detection, reentrant, actor, recovery, music],
            skipped: [],
        }
    );
}

fn classify_ai_detection_outbox(value: &AiDetectionOutbox) {
    classify_fields!(
        value,
        AiDetectionOutbox {
            persisted: [stimuli, mark_alerted],
            skipped: [],
        }
    );
}

fn classify_ai_reentrant_outbox(value: &AiReentrantOutbox) {
    classify_fields!(
        value,
        AiReentrantOutbox {
            persisted: [
                cross_npc_actions,
                // Persisted; nested `QueuedSelfStimulus::origin` is skipped.
                self_stimuli,
                owner_work,
                reconsider_approach_completion_pending,
                reconsider_approach_replaced_path_waiter,
                battle_observe_completion_pending,
                look_for_help_completion_pending,
                waypoint_script_reach_point,
                alert_soldier_completion_pending,
                dead_body_alert_completion_pending,
                tower_guard_alert_officer_completion_pending,
                civilian_report_alert_officer_completion_pending,
                brawl_hitting_completion_pending,
            ],
            skipped: [engine_drains_after_script_go_on],
        }
    );
}

fn classify_enemy_ai(value: &EnemyAi) {
    classify_fields!(
        value,
        EnemyAi {
            persisted: [
                base,
                pending_special_strike,
                pending_sword_strike_consideration,
                pending_combat_insult_after_strike_consideration,
                missed_pc,
                pc_missed,
                pc_gone_away_in_this_direction,
                frame_when_missed_charly,
                heard_nets,
                detected_something_there,
                investigating_distraction,
                last_seek_direction_index,
                beggar_to_examine,
                beggar_is_npc,
                current_task_priority,
                minimal_task_priority,
                new_task_priority,
                number_of_different_checkpoints,
                thirsty,
                position_change_locked_for_test,
                other_bodies_to_examine,
                beggars_to_control,
                positions_of_beggars_to_control,
                seen_dead_body,
                seeking_charly,
                my_seek_points,
                personal_seek_point_1,
                personal_seek_point_2,
                seek_center,
                actual_seek_point,
                seek_point_view_directions,
                seek_flags,
                old_odds,
                gather_position,
                gather_direction,
                gather_position_instructed,
                search_charly_way,
                officers_position,
                previous_state,
                previous_substate,
                reported_to_officer,
                missed_soldier_timer,
                old_money,
                other_seen_money,
                other_seen_ale,
                money_fight_enemies,
                money_fight_victims,
                archer_behind_me,
                shield_bearer_before_me,
                shield_bearer_direction,
                phalanx_aborted,
                changed_to_alert_path,
                already_seen_bodies,
                alerted_us,
                pending_alert_soldier_candidates,
                pending_group_instruction_candidates,
                pending_group_instruction_seek_flags,
                pending_group_instruction_clear_location_after_accept,
                my_shooting_point,
                my_archery_sector,
                my_archery_sector_index,
                my_archery_point_index,
                my_archery_point_increment,
                enemy_seen_below,
                enemy_had_this_elevation,
                known_enemy_strike_1,
                known_enemy_strike_2,
                known_enemy_strike_3,
                return_to_patrol_point,
                fleeing_seen_enemy_counter,
                // Persisted; nested `Stimulus::self_origin` is skipped.
                last_stimulus_dispatched_to_patrol,
                character_id,
                old_life_points,
                initial_life_points,
                list_them,
                ambush_point_array_reset,
                ambush_point_status,
                forced_next_battle_decision,
                reset_battle_decision,
                soldier_profile_iq,
                soldier_profile_courage,
                soldier_profile_shooting,
                soldier_profile_vip,
                soldier_profile_bee_time,
                soldier_profile_pride,
                soldier_profile_hearing_factor,
                soldier_profile_rank,
                soldier_profile_initiative,
                soldier_profile_beer,
                ale_reliable_distraction,
                soldier_profile_money,
                soldier_profile_apple,
                soldier_profile_whistle,
                soldier_profile_duty,
                soldier_profile_endurance,
                is_vip,
                sword_range,
                hth_weapon_id,
                sword_is_charge_weapon,
                next_sword_strike_frame,
                company_number,
                left_combat_neighbour,
                right_combat_neighbour,
                attentive,
                will_be_attentive,
                forced_attentive,
                guarded_pc,
                my_line_jump,
                tower_guard,
                combat_trainer,
                is_archer_unit,
            ],
            skipped: [],
        }
    );
}

fn classify_friendly_ai(value: &FriendlyAi) {
    classify_fields!(
        value,
        FriendlyAi {
            persisted: [
                base,
                beggar_dont_talk_counter,
                fleeing_seen_enemy_counter,
                wants_to_talk,
                last_talk_partner,
                can_go_away,
            ],
            skipped: [],
        }
    );
}

#[test]
fn persisted_ai_owner_fields_are_exhaustively_classified() {
    classify_ai_controller(&AiController::default());
    classify_ai_global_state(&AiGlobalState::default());
    classify_queued_self_stimulus(&QueuedSelfStimulus::from(StimulusType::EventDone));
    classify_stimulus(&Stimulus::new(StimulusType::EventDone));
    classify_ai_outbox(&AiOutbox::default());
    classify_ai_detection_outbox(&AiDetectionOutbox::default());
    classify_ai_reentrant_outbox(&AiReentrantOutbox::default());
    classify_enemy_ai(&EnemyAi::default());
    classify_friendly_ai(&FriendlyAi::default());
}
