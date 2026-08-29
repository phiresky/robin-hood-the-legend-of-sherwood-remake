//! `EnemyAi` event handlers: `think_unexpected_event`,
//! `think_alerting_event`, the per-event standard procedures, and
//! the post-event helpers (`get_angry_about_apple`,
//! `couldnt_reachpoint_emergency_routine`,
//! `event_sees_charly_standard_procedure`).
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the file manageable.

use crate::ai::*;
use crate::parameters_ai;

use super::util::{ai_max_norm_distance, ai_square_distance, enemy_is_below_me};
use super::{EnemyAi, ProfileRank, SeekFlags, UNDEFINED_DIRECTION, combat, task_priority};

fn good_strike_lifecycle_debug_matches(ctx: &AiContext) -> bool {
    if std::env::var_os("PARITY_DEBUG_GOOD_STRIKE_LIFECYCLE").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for GOOD_STRIKE diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_GOOD_STRIKE_FRAME").is_none_or(|expected| expected == ctx.frame)
        && parse_filter("PARITY_DEBUG_GOOD_STRIKE_CREATION_ORDER")
            .is_none_or(|expected| ctx.original_creation_order == Some(expected))
}

impl EnemyAi {
    /// Standard "I see the friend I was looking for" reaction.
    ///
    /// Three rank branches (officer / soldier / knight) inside
    /// `STATE_SEEKING`, followed by a common "reunion" tail that either
    /// kicks off a `DetectedCharly` wait, resumes a synchronised macro,
    /// or registers as a synchronising actor on the friend.  The rank
    /// branches can short-circuit the function before the tail ever runs.
    fn event_sees_charly_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        charly: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // `pCharly` metadata (rank, reported-to-officer, substate) comes
        // from the per-tick entity view.  If the view is missing we skip
        // the rank-specific branches and fall through to the reunion
        // tail — losing the view snapshot means we can't trust the rank
        // checks, but the reunion tail is a safe default.
        let charly_view = ctx.entity_view(charly).cloned();

        if self.base.current_state == AiState::Seeking {
            match self.get_rank() {
                // Officer branch.
                ProfileRank::Officer => {
                    // Ignore while already waiting / lecturing the charly
                    // we sent out.
                    if matches!(
                        self.base.current_substate,
                        Substate::SeekingOfficerWaitForCharly
                            | Substate::SeekingOfficerLectureCharly
                    ) {
                        return;
                    }
                    // Ignore if charly already reported.
                    let already_reported = charly_view
                        .as_ref()
                        .map(|v| v.reported_to_officer)
                        .unwrap_or(false);
                    if already_reported {
                        return;
                    }
                    self.base.outbox.actor.queue_unalert_near_charly_seekers(
                        CharlySeekerTarget::Npc(charly),
                        self.base.antagonist,
                    );

                    // If pCharly is a rank-Soldier, acquire him and wait.
                    let charly_is_soldier = charly_view
                        .as_ref()
                        .map(|v| v.is_soldier() && v.rank == ProfileRank::Soldier)
                        .unwrap_or(false);
                    if charly_is_soldier {
                        self.base.say(Remark::FoundCharly);
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: charly,
                                stimulus_type: StimulusType::CallGoToOfficer,
                                info: StimulusInfo::Human(self.base.me),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                        self.base.antagonist = charly;
                        self.base.face_entity(charly, ctx);
                        self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForCharly);
                        self.base.launch_timer(10, ctx.frame);
                        return;
                    }
                    // Fall through to reunion tail.
                }

                // Soldier branch.
                ProfileRank::Soldier => {
                    // Only if we have an antagonist (the officer who sent
                    // us out) and pCharly is an unreported soldier.
                    let has_antagonist = self.base.antagonist != 0;
                    let charly_ok = charly_view
                        .as_ref()
                        .map(|v| {
                            v.is_soldier()
                                && v.rank == ProfileRank::Soldier
                                && !v.reported_to_officer
                        })
                        .unwrap_or(false);
                    if has_antagonist && charly_ok {
                        self.seek_flags &= !SeekFlags::REPORT_OFFICER_AFTER;

                        // Branch on pCharly's substate.
                        let charly_substate = charly_view
                            .as_ref()
                            .map(|v| v.ai_substate)
                            .unwrap_or(Substate::None);
                        match charly_substate {
                            Substate::SeekingCharlySentToOfficer
                            | Substate::SeekingCharlyGoToOfficer
                            | Substate::SeekingCharlyGoToOfficerSeen
                            | Substate::SeekingCharlyGetLectureByOfficer
                            | Substate::SeekingCharlyGetLectureByOfficer2 => {
                                // Already sent to officer.
                                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                                return;
                            }
                            _ => {
                                // Send charly to officer ourselves.
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingSendCharlyToOfficer,
                                );
                                self.base.outbox.actor.queue_unalert_near_charly_seekers(
                                    CharlySeekerTarget::Npc(charly),
                                    self.base.antagonist,
                                );
                                // Original calls UnalertAllNearCharlySeekers
                                // synchronously before Say(FOUND_CHARLY).
                                // Preserve that statement boundary: a
                                // rejected speech can immediately dispatch
                                // EVENT_MYTALK_1 and ReturnToDuty, whose
                                // patrol-chief visibility query must not
                                // overtake the earlier Charly-seeker sweep.
                                self.base.outbox.reentrant.owner_work.push(
                                    AiOwnerWork::ActorEffects(std::mem::take(
                                        &mut self.base.outbox.actor,
                                    )),
                                );
                                self.base
                                    .say_with_flags(Remark::FoundCharly, SpeechFlags::MYTALK_1);
                                self.base
                                    .outbox
                                    .reentrant
                                    .owner_work
                                    .push(AiOwnerWork::ResumeSendCharlyAfterSpeech { charly });
                                return;
                            }
                        }
                    }
                    // Fall through to reunion tail.
                }

                // Knight branch — no-op, falls through.
                ProfileRank::Knight => {}

                ProfileRank::None => {}
            }

            // Say(REMARK_FOUND_CHARLY) inside the seeking block.
            self.base.say(Remark::FoundCharly);
        }

        // ── Reunion tail. ──────────────────────────────────────────────
        // Zero sorrow and clear the checkpoint charly.
        self.base.sorrow_level = 0;
        self.base.set_checkpoint_charly(0);

        // Branch on synchronize-index / sync-charly / macro state.
        let no_sync = self.base.synchronize_index == u16::MAX
            || self.base.synchronize_charly == 0
            || !self.base.macro_in_progress;
        if no_sync {
            // Plain reunion — halt, go green, face charly.
            self.base.outbox.actor.halt = true;
            self.set_alert_status(AlertLevel::Green);
            self.base.face_entity(charly, ctx);
            if self.base.current_state == AiState::Default {
                self.set_state(AiState::Default, Substate::DefaultDetectedCharly);
                self.base
                    .launch_timer(parameters_ai::AI_CHARLY_LOOK_TIME as u32, ctx.frame);
            } else {
                // Stash previous state, unalert seekers, transition to
                // SEEKING_DETECTED_CHARLY.
                self.previous_state = self.base.current_state as i32;
                self.previous_substate = self.base.current_substate as i32;
                self.base.outbox.actor.queue_unalert_near_charly_seekers(
                    CharlySeekerTarget::Npc(charly),
                    self.base.antagonist,
                );
                self.set_state(AiState::Seeking, Substate::SeekingDetectedCharly);
                self.base
                    .launch_timer(parameters_ai::AI_CHARLY_LOOK_TIME as u32, ctx.frame);
            }
            return;
        }

        // synchronize_charly is in STATE_DEFAULT?
        let sync_view = ctx.entity_view(self.base.synchronize_charly).cloned();
        let sync_in_default = sync_view
            .as_ref()
            .map(|v| v.ai_state == AiState::Default)
            .unwrap_or(false);
        if !sync_in_default {
            // "Forget it" — drop back into macro flow.
            self.set_state(AiState::Default, Substate::DefaultInMacro);
            self.base.execute_next_macro_command(sim, ctx);
            return;
        }

        // Check whether the sync friend is already at the sync waypoint.
        let friend_is_already_there = if let Some(v) = sync_view.as_ref() {
            if v.macro_in_progress {
                v.path_current_waypoint_index as u16 == self.base.synchronize_index
            } else if v.ai_substate == Substate::DefaultEnroute {
                v.path_last_waypoint_index as u16 == self.base.synchronize_index
            } else {
                false
            }
        } else {
            false
        };

        if friend_is_already_there {
            // Already at the sync waypoint — resume macro.
            self.set_state(AiState::Default, Substate::DefaultInMacro);
            self.base.execute_next_macro_command(sim, ctx);
        } else {
            // Wait — register ourselves and stall.
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::RegisterSynchronizingActor {
                    target: self.base.synchronize_charly,
                    actor: self.base.me,
                },
            );
            self.set_state(AiState::Default, Substate::DefaultSynchronizing);
            self.base.launch_timer(20, ctx.frame);
        }
    }

    // -----------------------------------------------------------------------
    // ThinkUnexpectedEvent
    // -----------------------------------------------------------------------

    pub(crate) fn think_unexpected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            // FilterAIEvent / EVENT_MISSES_CHARLY path: fire a fresh
            // `SearchCharly` whenever the unexpected-event dispatcher
            // receives a charly-missing stimulus and we aren't already in
            // the middle of a charly seek.
            StimulusType::EventMissesCharly => {
                let already_seeking = matches!(
                    (self.base.current_state, self.base.current_substate),
                    (AiState::Seeking, Substate::SeekingCharly)
                ) || self.seeking_charly;
                if !already_seeking {
                    self.search_charly(sim, global, ctx, tick, grid);
                }
                return true;
            }
            StimulusType::EventOutOfView => {
                if self.base.current_state == AiState::Attacking
                    && let StimulusInfo::Human(enemy) = stimulus.info
                {
                    tracing::trace!(
                        me = self.base.me,
                        frame = ctx.frame,
                        substate = ?self.base.current_substate,
                        enemy,
                        primary_target = self.base.primary_target,
                        enemy_seen_below = self.enemy_seen_below,
                        list_them = ?self.list_them,
                        "OUTOFVIEW while attacking"
                    );
                    // Lost sight of enemy while attacking.
                    match self.base.current_substate {
                        Substate::AttackingBowObservingLoading
                        | Substate::AttackingBowObserving
                        | Substate::AttackingBowShooting
                        | Substate::AttackingBowLoading
                        | Substate::AttackingBowAiming => {
                            // These five labels precede
                            // `_ANY_SWORDFIGHT_SUBSTATE_` in Original and
                            // deliberately fall through it unless the special
                            // below-target recovery consumes the event.
                            if self.enemy_seen_below {
                                self.reinitialize_them_list(ctx, tick);
                                return true;
                            }
                            if enemy == self.base.primary_target
                                && self.is_detecting_360_degrees(enemy, ctx)
                            {
                                return false;
                            }
                            // The swordfight labels in turn fall through the
                            // moving-combat stare-vector guard before the
                            // shared lost-enemy handler.
                            if self.enemy_is_behind_me(ctx) {
                                return false;
                            }
                            self.out_of_view_seek_handler(sim, enemy, global, ctx, tick, grid);
                        }

                        s if s.is_any_swordfight() => {
                            // _ANY_SWORDFIGHT_SUBSTATE_ 360° short-circuit
                            // — if the target is still within the NPC's
                            // real-radius "feel bubble" despite the cone
                            // LOS drop, the event is silently ignored and
                            // the NPC stays engaged. Without this check
                            // the port bailed every time the view cone
                            // flickered during `AttackingRunningToEnemy`,
                            // cycling the NPC Attacking→Seeking→Attacking
                            // every ~100 ms.
                            //
                            // NOTE: the previous port used
                            // `find_fighter(enemy, tick)` as the proxy, but
                            // `tick.nearby_fighters` is only populated on
                            // the primary NPC-detection dispatch path — the
                            // falling-edge EVENT_OUTOFVIEW dispatch built a
                            // `tick_data` from `AiPerTickData::stub()`,
                            // so `nearby_fighters` was empty and the check
                            // always failed.  Using the `entity_views`
                            // distance gate directly avoids that aliasing.
                            if enemy == self.base.primary_target
                                && self.is_detecting_360_degrees(enemy, ctx)
                            {
                                // Still close — stay in swordfight.
                                return false;
                            }
                            // Original's `_ANY_SWORDFIGHT_SUBSTATE_` case has
                            // no break here. A failed 360-degree check falls
                            // through the same stare-vector guard used by
                            // REACTIONTIME_RUNNING / APPROACH_TO_OBSERVE /
                            // ADVANCING_WITH_SHIELD before reaching the
                            // shared lost-enemy body.
                            if self.enemy_is_behind_me(ctx) {
                                return false;
                            }
                            self.out_of_view_seek_handler(sim, enemy, global, ctx, tick, grid);
                        }

                        // REACTIONTIME_RUNNING / APPROACH_TO_OBSERVE /
                        // ADVANCING_WITH_SHIELD run an "enemy behind me"
                        // check first — if the NPC is just looking the
                        // wrong way while moving, the dot product of
                        // (lookVector · stareVector) is negative and the
                        // event is silently dropped. Only when the stare
                        // is actually in front of the NPC do we fall
                        // through to the seek handler below.
                        Substate::AttackingReactiontimeRunning
                        | Substate::AttackingApproachToObserve
                        | Substate::AttackingAdvancingWithShield => {
                            if self.enemy_is_behind_me(ctx) {
                                // Just out of view because we're looking
                                // the wrong way — ignore the OUTOFVIEW.
                                return false;
                            }
                            // Fall through to the seek handler below by
                            // invoking the shared helper directly.
                            self.out_of_view_seek_handler(sim, enemy, global, ctx, tick, grid);
                        }

                        // Stationary / combat-posture substates. On
                        // EVENT_OUTOFVIEW, forecast the target's
                        // destination and either chase (via seek_area) or
                        // face + get_battle_overview.
                        //
                        // `ATTACKING_REACTIONTIME_TURNING` is explicitly
                        // excluded and falls to the default reinitialization
                        // branch. The running/walking/charging substates are
                        // members of Original's `_ANY_SWORDFIGHT_SUBSTATE_`
                        // macro and were handled by the earlier arm.
                        Substate::AttackingReactiontime
                        | Substate::AttackingQuittingSwordfight
                        | Substate::AttackingReserve
                        | Substate::AttackingLastReserve
                        | Substate::AttackingObserve
                        | Substate::AttackingObserveAndMove
                        | Substate::AttackingHitting
                        | Substate::AttackingProtectingWithShield
                        | Substate::AttackingPhalanx
                        | Substate::AttackingTooProudToAttack
                        | Substate::AttackingTooProudToAttackApproach => {
                            self.out_of_view_seek_handler(sim, enemy, global, ctx, tick, grid);
                        }

                        // Do-nothing substates.
                        Substate::AttackingTooProudToAttackRetire
                        | Substate::AttackingTooProudToAttackRetireTurn
                        | Substate::AttackingReactiontimeBending => {}

                        // Wait-for-avenger substates. Original
                        // (`RHartificialmalignity.cpp:5442`) sweeps around the
                        // waiting soldier itself (`Position(mpMe)`), not the
                        // remembered avenger position it is staring at, and
                        // takes the plain `GetBattleOverview()` default flags
                        // (0) rather than the FAST_OVERVIEW variant used by
                        // the sight/hearing entry points.
                        Substate::AttackingWaitForAvengerOnRoof => {
                            self.reinitialize_them_list(ctx, tick);
                            if self.list_them.is_empty() {
                                self.seek_area(
                                    sim,
                                    ctx.position,
                                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                                    SeekFlags::empty(),
                                    UNDEFINED_DIRECTION,
                                    global,
                                    ctx,
                                    tick,
                                );
                            } else {
                                self.get_battle_overview(0, ctx, tick);
                            }
                        }

                        _ => {
                            // Default — just reinitialize them list.
                            self.reinitialize_them_list(ctx, tick);
                        }
                    }
                }
            }

            StimulusType::EventCouldntReachPoint => {
                // Pathfinding failure.
                match self.base.current_substate {
                    // Seek point unreachable → try next.
                    Substate::SeekingSeekpoint => {
                        self.seek_next_point(sim, global, ctx, tick);
                    }
                    // Body unreachable → seek area.
                    Substate::SeekingBody => {
                        if !self.examine_other_bodies(ctx, tick) {
                            self.seek_area(
                                sim,
                                ctx.position,
                                parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                SeekFlags::empty(),
                                UNDEFINED_DIRECTION,
                                global,
                                ctx,
                                tick,
                            );
                        }
                    }
                    Substate::AttackingObserve => {
                        // Ignore.
                    }
                    Substate::AttackingRunningToLadder
                        if stimulus.self_origin
                            == crate::ai::SelfStimulusOrigin::EngineCompletion
                            && self.base.timer_is_running
                            && self.base.substate_at_last_timer_launch
                                == Substate::AttackingRunningToLadder
                            && self.base.when_does_timer_ring == ctx.frame.saturating_add(30) =>
                    {
                        // This is specifically the engine-completion bridge,
                        // not an Original movement condolation. The latter
                        // enters this handler with Condolation provenance and
                        // must take the generic default arm below.
                        //
                        // The lift-entry GoNear in ReconsiderEnemyApproach is
                        // followed immediately by LaunchTimer and return
                        // (`RHartificialmalignity.cpp:6874-6898`). Control then
                        // returns through AttackEnemy to DECISION_FIGHT, whose
                        // couldn't-reachpoint arm switches to DECISION_OBSERVE
                        // (`RHartificialmalignity.cpp:7830-7841`). The failed
                        // observe route takes the inline avenger-on-roof
                        // fallback at lines 7973-7990. DECISION_FIGHT has not
                        // registered its log line at this source point; the
                        // lift branch's 30-frame timer is its exact surviving
                        // provenance. Rust learns the first route result only
                        // at this owner boundary, so resume that source-ordered
                        // failure tail here.
                        let target_position = ctx
                            .entity_view(self.base.primary_target)
                            .unwrap_or_else(|| {
                                panic!(
                                    "ladder route-failure target {} disappeared",
                                    self.base.primary_target
                                )
                            })
                            .position;
                        let target = self.base.primary_target;
                        let avenger_wait_position =
                            tick.avenger_wait_position_for(self.base.primary_target);
                        self.base.couldnt_reachpoint = true;
                        if avenger_wait_position.is_some() {
                            self.resume_reconsider_enemy_approach_after_go_near(
                                target_position,
                                avenger_wait_position,
                                ctx,
                            );
                            // Original constructs and settles this roof GoNear
                            // before DECISION_OBSERVE returns. Route the typed
                            // actor effects through the existing synchronous
                            // owner boundary so its actual verdict is visible
                            // to this frame's EndThink.
                            if self.base.outbox.actor.has_boundary_work() {
                                self.base.outbox.reentrant.owner_work.push(
                                    crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                        &mut self.base.outbox.actor,
                                    )),
                                );
                            }
                        } else {
                            // DECISION_FIGHT clears the failed lift approach
                            // and changes to DECISION_OBSERVE. Its observe
                            // GoNear fails synchronously too in this no-roof
                            // case, so the following source tail installs
                            // ApproachToObserve/timer 50 while retaining the
                            // failure latch for EndThink's generic overview.
                            self.resume_battle_observe_after_go_near(
                                target,
                                target_position,
                                None,
                                ctx,
                            );
                        }
                    }
                    Substate::AttackingApproachToObserve
                        if self.base.ai_log.iter().rev().any(|line| {
                            line.frame == ctx.frame
                                && line.line_type == LogLineType::BattleDecision
                                && line.info == Decision::Observe as u16
                        }) =>
                    {
                        // A same-frame failure here is the delayed result of
                        // DECISION_OBSERVE's GoNear. Original constructs the
                        // route inside that statement and tests
                        // mbCouldntReachpoint immediately after SetState;
                        // Rust can only discover a local Move failure after
                        // the typed tail has entered ApproachToObserve.
                        // Resume that source-local roof fallback instead of
                        // letting the staging delay turn it into the generic
                        // attacking emergency overview.
                        if let Some(wait_position) =
                            tick.avenger_wait_position_for(self.base.primary_target)
                        {
                            let target_position = ctx
                                .entity_view(self.base.primary_target)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "observe route-failure target {} disappeared",
                                        self.base.primary_target
                                    )
                                })
                                .position;
                            self.go_near(
                                AiState::Attacking,
                                Substate::AttackingRunToAvengerOnRoof,
                                wait_position,
                                50,
                                GotoFlags::RUN,
                                ctx,
                            );
                            self.base.seek_position = target_position;
                        } else {
                            self.couldnt_reachpoint_emergency_routine(sim, global, ctx, tick);
                        }
                    }
                    Substate::FleeingPanic => {
                        // Original routes a failed panic-run movement back
                        // through the shared FLEEING_PANIC state machine.
                        // The generic emergency routine would instead return
                        // a fleeing soldier to duty and discard the remaining
                        // panic runs.
                        self.base
                            .think_expected_event_common_stuff(sim, stimulus, ctx);
                    }
                    _ => {
                        self.couldnt_reachpoint_emergency_routine(sim, global, ctx, tick);
                    }
                }
            }

            StimulusType::EventImpossible => {
                // Impossible generic actions are treated as done so the AI does not
                // abandon its current high-level behavior. Returning to duty
                // here can leave a soldier's swordfight opponent list intact
                // while its AI state falls back to patrol.
                if self.base.current_substate == Substate::AttackingKillingSleepingEnemy {
                    self.get_battle_overview(0, ctx, tick);
                } else {
                    let done = Stimulus::new(StimulusType::EventDone);
                    self.think(sim, &done, global, ctx, tick, grid);
                }
            }

            StimulusType::EventFitAgain => {
                // Recovered from unconsciousness.
                //
                // Engine-facing calls share the owner-work FIFO with
                // SetState so their exact intra-Think order survives the
                // temporary Rust borrow boundary.
                // The money-fight branch routes to `return_to_duty` and
                // clears `knocked_out_in_money_fight` so the victor
                // cleanly rejoins their duty loop instead of getting stuck
                // in `SleepingAwakening`.
                if self.base.current_substate != Substate::SleepingUnconscious {
                    // The dispatch only fires from SLEEPING_UNCONSCIOUS;
                    // any other substate falls through as a no-op.
                    return false;
                }

                let knocked_out_in_money_fight = self.base.knocked_out_in_money_fight;
                self.base.outbox.reentrant.owner_work.push(
                    crate::ai::AiOwnerWork::RestoreDetectableObjects {
                        knocked_out_in_money_fight,
                    },
                );
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::InformResurrection);
                self.base.clear_emoticon();

                if knocked_out_in_money_fight {
                    self.base.knocked_out_in_money_fight = false;
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else {
                    self.set_state(AiState::Sleeping, Substate::SleepingAwakening);
                    self.base.outbox.reentrant.owner_work.push(
                        crate::ai::AiOwnerWork::LaunchTimer {
                            frames: parameters_ai::AI_WAKEUP_IDLING_TIME as u32,
                            current_frame: ctx.frame,
                        },
                    );
                    self.base.outbox.reentrant.owner_work.push(
                        crate::ai::AiOwnerWork::SetEyeStatus(
                            crate::element::EyeStatus::LookForward,
                        ),
                    );
                }
            }

            StimulusType::EventQuitSwordfight => {
                // Only react if in a real swordfight substate (not
                // approach/run).
                if self.base.current_substate.is_real_swordfight() {
                    // In Merry Man Forest, try to flee via
                    // MerryManForestCassos. Only proceed with the normal
                    // quit transition if NOT forest or if flee failed.
                    if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global)
                    {
                        self.set_state(AiState::Attacking, Substate::AttackingQuittingSwordfight);
                        // EVENT_QUIT_SWORDFIGHT calls both reciprocal Update*
                        // setters in Original (RHartificialmalignity.cpp:
                        // 5618-5619).
                        self.clear_combat_neighbours();
                        self.base.launch_timer(3, ctx.frame);
                    }
                }
            }

            StimulusType::EventSwordStrike => {
                if matches!(
                    self.base.current_substate,
                    Substate::AttackingSwordfight
                        | Substate::AttackingSwordfightSpecialStrike
                        | Substate::AttackingMovingAroundOldEnemy
                        | Substate::AttackingApproachingNewEnemy
                ) {
                    let StimulusInfo::Human(attacker) = stimulus.info else {
                        panic!("EVENT_SWORDSTRIKE requires its human attacker")
                    };
                    self.base
                        .outbox
                        .reentrant
                        .owner_work
                        .push(crate::ai::AiOwnerWork::ConsiderToBeginParade { attacker });
                }
            }

            StimulusType::EventSeesSoldier => {
                // EVENT_SEES_SOLDIER: soldier-spotting-fellow-soldier →
                // "go tell the officer" / "call this soldier over"
                // coordination flow.
                let StimulusInfo::Human(antagonist) = stimulus.info else {
                    return false;
                };

                // State/substate `bReact` gate.
                let react = match self.base.current_state {
                    AiState::Default => true,
                    AiState::Seeking => matches!(
                        self.base.current_substate,
                        Substate::SeekingOfficerLookingForSoldiers1
                            | Substate::SeekingOfficerLookingForSoldiers1Sidewards
                            | Substate::SeekingOfficerLookingForSoldiers2
                            | Substate::SeekingOfficerLookingForSoldiers2Sidewards
                            | Substate::SeekingOfficerLookingForSoldiers3
                            | Substate::SeekingOfficerLookingForSoldiers3Sidewards
                            | Substate::SeekingRunningToOfficer
                    ),
                    _ => false,
                };
                if !react {
                    return false;
                }

                self.base.antagonist = antagonist;
                let antagonist_cs = tick.camp_soldiers.iter().find(|cs| cs.handle == antagonist);

                match self.get_rank() {
                    ProfileRank::Soldier => {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::RequestAlert {
                                target: antagonist,
                                caller: self.base.me,
                                continuation: crate::ai::AlertContinuation::SoldierSawOfficer,
                            },
                        );
                    }
                    ProfileRank::Officer => {
                        // Officer sees soldier → assert that the seen
                        // target is a soldier, gate on
                        // `CanCallThisSoldier`, then face + transition
                        // into the SeekingOfficerCallSoldier handshake.
                        let cs = antagonist_cs.unwrap_or_else(|| {
                            panic!(
                                "officer {} EVENT_SEES_SOLDIER requires target {} in camp soldier roster",
                                self.base.me, antagonist
                            )
                        });
                        assert_eq!(
                            cs.rank,
                            ProfileRank::Soldier,
                            "officer {} EVENT_SEES_SOLDIER target {} must have soldier rank",
                            self.base.me,
                            antagonist
                        );
                        if self.can_call_this_soldier(cs, ctx, tick) {
                            self.face_npc(antagonist, ctx);
                            // Transition to
                            // SUBSTATE_SEEKING_OFFICER_CALL_SOLDIER — the
                            // EventDone arm of that substate sends
                            // CALL_HEY and launches the WaitForSoldier
                            // handshake.
                            self.set_state(AiState::Seeking, Substate::SeekingOfficerCallSoldier);
                            // DeleteAllDetectables(FRIEND) — committed to
                            // this soldier, drop the rest of the friend
                            // list so further EVENT_SEES_SOLDIER calls
                            // don't pre-empt.
                            self.base
                                .outbox
                                .actor
                                .delete_detectables
                                .push(crate::element::DetectableType::Friend);
                        }
                    }
                    ProfileRank::Knight | ProfileRank::None => {
                        // Knights never reach EVENT_SEES_SOLDIER in the
                        // patrol-coordination flow.
                    }
                }
            }

            StimulusType::CallAlert => {
                match stimulus.info {
                    StimulusInfo::Hint(ref hint) => {
                        self.base.seek_position = hint.seek_point;
                        self.base
                            .my_reconnaissance_report
                            .update(ReportType::Enemy, hint.seek_point);
                        // React based on rank
                        match self.get_rank() {
                            ProfileRank::Officer => {
                                self.base.friends_are_alerted = true;
                                self.alert_soldiers(
                                    hint.seek_point,
                                    0,
                                    global,
                                    grid,
                                    ctx,
                                    tick,
                                    AlertSoldiersFailureContinuation::None,
                                );
                            }
                            _ => {
                                self.current_task_priority = task_priority::ALERT;
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingHeardstepsReactiontime,
                                );
                                self.base.face_position(hint.seek_point);
                                self.react(
                                    parameters_ai::AI_MAX_ALERT_REACTIONTIME as u16,
                                    ctx,
                                    tick,
                                );
                            }
                        }
                    }
                    // Civilian-sourced CALL_ALERT — a civilian ran to this
                    // soldier and wants to hand over a report. Accept iff
                    // in STATE_DEFAULT, else return false ("Sorry, dear
                    // civilian, I have no time for you").  Transition to
                    // SEEKING_WAIT_FOR_ALERTING_CIVILIAN, face the
                    // civilian, launch a 20-frame reaction timer, set a
                    // transient ? emoticon.
                    StimulusInfo::Human(civilian) => {
                        let caller = ctx.entity_view(civilian).unwrap_or_else(|| {
                            panic!(
                                "CALL_ALERT recipient {} requires caller {} entity view",
                                self.base.me, civilian
                            )
                        });
                        // Original assigns mpAntagonist before deciding whether the
                        // caller can be heard. A rejected civilian report therefore
                        // still replaces the actor tracked by the current behavior.
                        self.base.antagonist = civilian;
                        if caller.is_civilian() {
                            if self.base.current_state != AiState::Default {
                                return false;
                            }
                            // Original CALL_ALERT's civilian arm calls the actor's
                            // `Halt()` directly, not AI `StopAll()`.  The actor work
                            // must be interrupted before the state callback, while an
                            // in-flight waypoint macro and its macro timer survive.
                            // See RHartificialmalignity.cpp, ThinkUnexpectedEvent's
                            // CALL_ALERT / civilian branch.
                            self.base.outbox.actor.queue_halt();
                            self.base.face_entity(civilian, ctx);
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingWaitForAlertingCivilian,
                            );
                            self.base.launch_timer(20, ctx.frame);
                            self.base.set_transient_emoticon(
                                EmoticonType::QuestionMark,
                                20,
                                ctx.frame,
                            );
                            return true;
                        }
                        match self.get_rank() {
                            ProfileRank::Soldier => {
                                let react = matches!(
                                    self.base.current_state,
                                    AiState::Default | AiState::Wondering
                                ) || self.base.current_state == AiState::Seeking
                                    && matches!(
                                        self.base.current_substate,
                                        Substate::SeekingSoldierGiveReportToOfficer
                                            | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                                            | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                                            | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd
                                    );
                                if !react
                                    || !self.answer_question(Question::HasTheNewTaskPriority, ctx)
                                {
                                    return false;
                                }
                                assert_eq!(
                                    caller.rank,
                                    ProfileRank::Officer,
                                    "soldier CALL_ALERT caller must be an officer"
                                );
                                // Original's soldier-from-officer CALL_ALERT arm calls
                                // `mpMe->Halt()` directly, not AI `StopAll()`. Halt
                                // interrupts actor work but leaves an in-flight waypoint
                                // macro and its macro timer intact.
                                self.base.outbox.actor.queue_halt();
                                self.current_task_priority = self.new_task_priority;
                                self.gather_position_instructed = false;
                                self.base.friends_are_alerted = true;
                                self.officers_position = caller.position;
                                self.base.face_position_3d_with_ctx(caller.position, ctx);
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingGroupCalledByOfficer,
                                );
                                self.base.launch_timer(20, ctx.frame);
                                self.base.set_transient_emoticon(
                                    EmoticonType::QuestionMark,
                                    20,
                                    ctx.frame,
                                );
                                return true;
                            }
                            ProfileRank::Officer => {
                                let react = self.base.current_state == AiState::Default
                                    || self.base.current_state == AiState::Seeking
                                        && matches!(
                                            self.base.current_substate,
                                            Substate::SeekingOfficerWaitForInstructedGroup
                                                | Substate::SeekingOfficerWaitForInstructedSoldier
                                        );
                                if !react {
                                    return false;
                                }
                                assert_eq!(
                                    caller.rank,
                                    ProfileRank::Soldier,
                                    "officer CALL_ALERT caller must be a soldier"
                                );
                                // Original's officer-from-soldier CALL_ALERT arm also
                                // calls `mpMe->Halt()` directly. In particular, it does
                                // not route through AI `StopAll()` and must not break an
                                // in-flight waypoint macro or its macro timer.
                                self.base.outbox.actor.queue_halt();
                                self.base.friends_are_alerted = true;
                                self.base.face_entity(civilian, ctx);
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingOfficerWaitForAlertingSoldier,
                                );
                                self.base.launch_timer(20, ctx.frame);
                                self.base.set_transient_emoticon(
                                    EmoticonType::QuestionMark,
                                    20,
                                    ctx.frame,
                                );
                                return true;
                            }
                            ProfileRank::Knight | ProfileRank::None => {
                                panic!(
                                    "CALL_ALERT reached unsupported recipient rank {:?}",
                                    self.get_rank()
                                )
                            }
                        }
                    }
                    _ => {}
                }
            }

            StimulusType::CallCombatAlert => {
                if self.get_rank() != ProfileRank::Soldier {
                    panic!(
                        "CALL_COMBAT_ALERT reached unsupported recipient rank {:?}",
                        self.get_rank()
                    );
                }
                match self.base.current_state {
                    AiState::Default | AiState::Wondering | AiState::Seeking => {
                        let StimulusInfo::Position(ref pos) = stimulus.info else {
                            return false;
                        };
                        self.call_combat_alert_standard_procedure(pos, ctx, tick);
                        return true;
                    }
                    AiState::Attacking => return true,
                    _ => return false,
                }
            }

            StimulusType::CallGoToOfficer => {
                let StimulusInfo::Human(officer) = stimulus.info else {
                    return false;
                };
                if self.base.current_state != AiState::Default
                    && self.base.current_substate != Substate::SleepingAwakening
                {
                    return false;
                }
                self.base.antagonist = officer;
                self.set_state(AiState::Seeking, Substate::SeekingCharlySentToOfficer);
                self.base.set_emoticon(EmoticonType::None);
                self.base.launch_timer(30, ctx.frame);
                self.reported_to_officer = true;
                return true;
            }

            // Officer hails a soldier. The legacy implementation civilian branch asserts
            // false; CALL_HEY is only dispatched from
            // `SeekingOfficerCallSoldier` with the officer as sender.
            // Soldier accepts the call only if the new task priority
            // outranks the current one.
            StimulusType::CallHey => {
                let StimulusInfo::Human(officer) = stimulus.info else {
                    return false;
                };
                // Skip the civilian path (asserted away upstream).
                if let Some(view) = ctx.entity_view(officer)
                    && view.is_civilian()
                {
                    tracing::warn!(
                        "EnemyAi::think_unexpected_event: CALL_HEY from civilian unhandled \
                         (asserted away) — origin {officer}"
                    );
                    return false;
                }
                self.base.antagonist = officer;

                // React gate.
                let react = match self.base.current_state {
                    AiState::Default | AiState::Wondering => true,
                    AiState::Seeking => matches!(
                        self.base.current_substate,
                        Substate::SeekingRunningToOfficer
                            | Substate::SeekingRunningToOfficerSeen
                            | Substate::SeekingHeardstepsReactiontime
                            | Substate::SeekingBodyReactiontime
                    ),
                    _ => false,
                };
                if !react {
                    return false;
                }

                // Rank dispatch. RANK_OFFICER / RANK_KNIGHT are asserted
                // away upstream — only soldiers receive CALL_HEY.
                if self.get_rank() != ProfileRank::Soldier {
                    tracing::warn!(
                        "EnemyAi::think_unexpected_event: CALL_HEY at non-soldier rank \
                         {:?} (asserted away upstream)",
                        self.get_rank()
                    );
                    return false;
                }

                // Gate on Q_HAS_THE_NEW_TASK_PRIORITY.
                if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
                    return false;
                }

                self.current_task_priority = self.new_task_priority;
                self.base.stop_all();
                self.base.face_entity(officer, ctx);
                self.set_state(AiState::Seeking, Substate::SeekingSoldierCalledByOfficer);
                self.base.launch_timer(20, ctx.frame);
                self.base
                    .set_transient_emoticon(EmoticonType::QuestionMark, 20, ctx.frame);
                return true;
            }

            StimulusType::EventWaspAway => {
                if self.base.current_substate == Substate::WonderingWaspInArmour {
                    // Wasp finally clears, soldier slowly opens eyes (view
                    // cone grows from radius 5 back to standard), sets QM
                    // emoticon, blinks the enemy, and timers 30 frames
                    // before reacquiring.
                    self.base.outbox.actor.slowly_open_eyes = true;
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1);
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            StimulusType::EventNetAway => {
                if self.base.current_substate == Substate::WonderingUnderNet {
                    self.base.outbox.recovery.set_eye_status =
                        Some(crate::element::EyeStatus::LookForward);
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1);
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            StimulusType::EventAdversaryWeak => {
                if self.base.current_substate.is_any_swordfight() {
                    self.reconsider_swordfight(sim, true, global, ctx, tick, grid);
                }
            }

            // Special-strike gloating remark. Original guards on the
            // observable special-strike substate.
            StimulusType::EventGoodStrike => {
                let will_say =
                    self.base.current_substate == Substate::AttackingSwordfightSpecialStrike;
                let debug = good_strike_lifecycle_debug_matches(ctx);
                if debug {
                    eprintln!(
                        "[GOOD_STRIKE frame={} owner={} owner_co={:?} phase=think_entry state={:?} substate={:?} will_say={} vip={}]",
                        ctx.frame,
                        self.base.me,
                        ctx.original_creation_order,
                        self.base.current_state,
                        self.base.current_substate,
                        will_say,
                        self.is_vip,
                    );
                }
                if will_say {
                    let remark = if self.is_vip {
                        Remark::VipGoodStrikeCombat
                    } else {
                        Remark::GoodStrikeCombat
                    };
                    self.base.say(remark);
                    if debug {
                        eprintln!(
                            "[GOOD_STRIKE frame={} owner={} owner_co={:?} phase=say_queued remark={:?}]",
                            ctx.frame, self.base.me, ctx.original_creation_order, remark,
                        );
                    }
                }
            }
            // Kill remark.
            StimulusType::EventLethalStrike => {
                if self.base.current_substate == Substate::AttackingSwordfightSpecialStrike {
                    let remark = if self.is_vip {
                        Remark::VipVictory
                    } else {
                        Remark::KilledAdversary
                    };
                    self.base.say(remark);
                }
            }

            StimulusType::EventSeesBeggar => {
                // When in a seek-area substate, queue the beggar for later
                // identification (approach → identify1 → identify2).
                if let StimulusInfo::Human(beggar) = stimulus.info
                    && self.base.current_substate.is_seek_area()
                {
                    if beggar != self.beggar_to_examine {
                        tracing::debug!(
                            beggar,
                            substate = ?self.base.current_substate,
                            "EventSeesBeggar: queued beggar for identification"
                        );
                        // Queue beggar for control during seek_next_point(sim, ).
                        // Stores the beggar's actual position via the
                        // antagonist's position. We read it from the
                        // `ctx.antagonist` snapshot populated by the engine
                        // when it dispatched this stimulus.
                        self.beggars_to_control.push(beggar);
                        let beggar_pos = ctx
                            .antagonist
                            .as_ref()
                            .map(|a| a.position)
                            .unwrap_or(self.base.seek_position);
                        self.positions_of_beggars_to_control.push(beggar_pos);
                        self.base
                            .set_transient_emoticon(EmoticonType::QuestionMark, 20, 0);
                    }

                    // DeleteDetectableForAllNPC(beggar, DETECTABLE_BEGGAR)
                    // is outside Original's `beggar != mpBeggarToExamine`
                    // queueing guard. A repeated view while approaching the
                    // claimed beggar must therefore still scrub every NPC's
                    // BEGGAR list synchronously through the engine drain.
                    self.base.outbox.actor.delete_beggar_for_all_npc.push(
                        ctx.entity_id(beggar).unwrap_or_else(|| {
                            panic!("EventSeesBeggar target {beggar} has no typed live entity view")
                        }),
                    );
                }
            }

            StimulusType::EventEnemyNear => {
                tracing::trace!(
                    me = self.base.me,
                    frame = ctx.frame,
                    substate = ?self.base.current_substate,
                    "EventEnemyNear received"
                );
                // Original: RHArtificialMalignity::ThinkUnexpectedEvent,
                // EVENT_ENEMY_NEAR. The sender owns trainer/time gates; this
                // arm assigns the stimulus human and enters swordfight.
                let StimulusInfo::Human(enemy) = stimulus.info else {
                    tracing::warn!(
                        me = self.base.me,
                        info = ?stimulus.info,
                        "EventEnemyNear received without a human target"
                    );
                    return false;
                };
                match self.base.current_substate {
                    Substate::AttackingReactiontimeTurning
                    | Substate::AttackingReactiontime
                    | Substate::AttackingApproachToObserve
                    | Substate::AttackingObserve => {
                        self.base.primary_target = enemy;
                        self.begin_swordfight(ctx, tick);
                    }
                    _ => {}
                }
            }

            // EVENT_AFTER_SCRIPT_GO_ON. Drain the buffered stimulus queue
            // (stimuli enqueued by `start_think` while `script_locked` was
            // set), bailing early if any further script/locks state is
            // still active. Then on STATE_DEFAULT, advance the cached
            // patrol path one waypoint and resume from there so soldiers
            // continue from where the script left them rather than
            // restarting the patrol.
            StimulusType::EventAfterScriptGoOn => {
                if self.base.outbox.reentrant.engine_drains_after_script_go_on {
                    return false;
                }
                while !self.base.stimulus_queue.is_empty() {
                    if !self.base.locks_flag_field.is_empty() || self.base.script_locked {
                        return false;
                    }
                    let queued = self.base.stimulus_queue.remove(0);
                    if queued.stimulus_type != StimulusType::EventAfterScriptGoOn {
                        self.think(sim, &queued, global, ctx, tick, grid);
                    }
                }

                if self.base.current_state == AiState::Default {
                    let hiking_paths = &ctx.hiking_paths;
                    let advanced_dest = if let Some(ref mut path) = self.base.patrol_path {
                        path.advance();
                        path.current_waypoint(hiking_paths).map(|wp| Position {
                            x: wp.x as f32,
                            y: wp.y as f32,
                            sector: ctx.hiking_waypoint_sector(
                                usize::from(path.hiking_path_index),
                                usize::from(path.current_waypoint_index),
                                wp.sector,
                            ),
                            level: wp.level,
                        })
                    } else {
                        None
                    };
                    if let Some(dest) = advanced_dest {
                        let flags = self.base.default_path_walking_flags;
                        self.go_to(AiState::Default, Substate::DefaultEnroute, dest, flags, ctx);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                    return false;
                }
            }

            StimulusType::EventObjectAway => {
                // Dispatch on object type. `StolenObject` carries the
                // object handle but not its type; we match PURSE/COIN by
                // checking whether the stolen object is tracked as
                // money-of-interest (`interesting_object` or appears in
                // `other_seen_money`). Anything else — including the ALE
                // branch and the default assert + ReturnToDuty — falls
                // through to `return_to_duty`.
                if let StimulusInfo::Stolen(stolen) = stimulus.info {
                    let obj = stolen.object;
                    let thief = stolen.thief;
                    let is_money_of_interest = obj != 0
                        && (obj == self.base.interesting_object
                            || self.other_seen_money.contains(&obj));
                    if is_money_of_interest {
                        self.stolen_money_standard_procedure(thief, ctx, tick);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            StimulusType::CallPatrolCoordinate => {
                self.coordinate_patrol(&stimulus.info, ctx, tick.patrol_chief_position);
            }

            // The officer who
            // broke up the brawl tells the berated soldier to clean up
            // the KO'd friends by rousing them.
            StimulusType::CallCleanUpAfterBrawl => {
                if self.base.current_substate
                    == Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl
                {
                    self.create_list_of_near_money_fight_victims(ctx, tick);
                    self.awake_next_money_fight_victim_if_any(sim, ctx, tick);
                }
            }

            // EVENT_SEES_CHARLY dispatch guard: only react when seeking, or
            // while the looking-for-charly default substates are running.
            // Dispatches to the standard-procedure port below.
            StimulusType::EventSeesCharly => {
                if let StimulusInfo::Human(charly) = stimulus.info {
                    let eligible = self.base.current_state == AiState::Seeking
                        || self.base.current_substate == Substate::DefaultLookingForCharly
                        || self.base.current_substate == Substate::DefaultLookingSidewardsForCharly;
                    if eligible {
                        self.event_sees_charly_standard_procedure(sim, charly, ctx, tick);
                    }
                }
            }

            // A soldier returning from a callout reports back to the officer
            // ("Mr. Officer, I am back"). Officer transitions to
            // WaitForCharly and bumps an X-mark emoticon; if already in
            // WaitForCharly we acknowledge silently.
            StimulusType::CallMrOfficerIAmBack => {
                let StimulusInfo::Human(soldier) = stimulus.info else {
                    return false;
                };
                self.base.antagonist = soldier;

                // Dispatch on current state/substate.
                if self.base.current_state == AiState::Seeking
                    && self.base.current_substate == Substate::SeekingOfficerWaitForCharly
                {
                    return true;
                }
                let react = match self.base.current_state {
                    AiState::Default => true,
                    AiState::Seeking => matches!(
                        self.base.current_substate,
                        Substate::SeekingOfficerWaitForInstructedGroup
                            | Substate::SeekingOfficerWaitForInstructedSoldier
                    ),
                    _ => false,
                };
                if !react {
                    return false;
                }

                self.base.outbox.actor.halt = true;
                self.base.face_entity(soldier, ctx);
                self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForCharly);
                self.base.say(Remark::FoundCharly);
                self.base.launch_timer(20, ctx.frame);
                self.base
                    .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
                return true;
            }

            // A charly the chief was tracking just walked back into view.
            // Clear the checkpoint and watch them resurrect; outside the
            // eligible substate set the charly memory still gets cleared
            // (default arm).
            StimulusType::CallCharlyIsBack => {
                let StimulusInfo::Human(charly) = stimulus.info else {
                    return false;
                };
                let s = self.base.current_substate;
                let in_eligible_substate = s.is_seek_area()
                    || matches!(
                        s,
                        Substate::SeekingSoldierReturnToOfficer
                            | Substate::SeekingSoldierGiveReportToOfficer
                            | Substate::SeekingBodyReactiontime
                            | Substate::SeekingBody
                            | Substate::SeekingNet
                            | Substate::SeekingGroupGetInstructedByOfficer
                    );
                if in_eligible_substate {
                    if self.base.my_reconnaissance_report.charly == charly {
                        self.base.set_checkpoint_charly(0);
                        self.base.face_entity(charly, ctx);
                        self.base.clear_emoticon();
                        self.seek_flags &= !SeekFlags::REPORT_OFFICER_AFTER;
                        self.set_state(AiState::Seeking, Substate::SeekingLookingResurrectedCharly);
                        // Dead/unconscious charly gets a long stare; a
                        // healthy one only the standard 20.
                        let timer = ctx
                            .entity_view(charly)
                            .map(|v| v.is_dead || v.is_unconscious)
                            .unwrap_or(false);
                        self.base
                            .launch_timer(if timer { 200 } else { 20 }, ctx.frame);
                    }
                } else {
                    // Default arm: even when we can't react, drop the
                    // stale checkpoint so the chief doesn't keep nagging
                    // about a charly that's home.
                    self.base.set_checkpoint_charly(0);
                }
            }

            // Officer notices a soldier in a brawl. Dispatched from the
            // soldier-side brawl detection. Drunken officers skip straight
            // to BrawlReactiontime instead of the proper OfficerSeeingBrawl
            // pose.
            StimulusType::EventSeesBrawl => {
                if self.base.current_state != AiState::Default {
                    return false;
                }
                let StimulusInfo::Human(friend) = stimulus.info else {
                    return false;
                };
                self.base.stop_all();
                self.base.say(Remark::OfficerSeesBrawl);
                self.base.friend_in_trouble = friend;
                self.base.face_entity(friend, ctx);
                self.base.set_emoticon(EmoticonType::QuestionMark);
                let next = if self.base.blood_alcohol == 0 {
                    Substate::WonderingOfficerSeeingBrawl
                } else {
                    Substate::WonderingBrawlReactiontime
                };
                self.set_state(AiState::Wondering, next);
                self.base.launch_timer(30, ctx.frame);
            }

            // Officer tells a soldier brawling for money to stop. Receiver
            // halts, drops the coin memory, and switches to
            // looking-at-the-officer.
            StimulusType::CallFinishBrawl => {
                let s = self.base.current_substate;
                if !(s.is_take_money() || s.is_fight_for_money()) {
                    return false;
                }
                let StimulusInfo::Human(officer) = stimulus.info else {
                    return false;
                };
                self.base.stop_all();
                self.base.face_entity(officer, ctx);
                self.base.clear_emoticon();
                self.base.antagonist = officer;
                self.forget_all_nearby_coins(ctx);
                self.set_state(
                    AiState::Wondering,
                    Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl,
                );
                // LaunchTimer(300 + (rand() % 32)).
                let extra =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SoldierBrawlCooldown, 0..32);
                self.base.launch_timer(300 + extra, ctx.frame);
            }

            // Taking damage mid-swordfight: stop swinging, re-evaluate the
            // fight, and (if still actually swordfighting) bark a combat
            // insult.
            StimulusType::EventAfterCombatInjury => {
                if self.base.current_substate.is_real_swordfight() {
                    self.base.stop_all();
                    self.reconsider_swordfight(sim, false, global, ctx, tick, grid);
                    if self.base.current_substate == Substate::AttackingSwordfight {
                        if self.pending_sword_strike_consideration {
                            self.pending_combat_insult_after_strike_consideration = true;
                        } else {
                            self.base.say(Remark::CombatInsult);
                        }
                    }
                }
            }

            _ => {
                tracing::trace!(
                    "EnemyAi::think_unexpected_event: unhandled {:?} in {:?}",
                    stimulus_type,
                    self.base.current_substate,
                );
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // ThinkAlertingEvent
    // -----------------------------------------------------------------------

    pub(super) fn think_alerting_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventView => {
                if let StimulusInfo::Human(enemy) = stimulus.info {
                    match self.base.current_state {
                        AiState::Sleeping => {} // ignore (should not happen)
                        AiState::Wondering | AiState::Default | AiState::Seeking => {
                            if !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            ) {
                                self.event_view_standard_procedure(
                                    sim, enemy, global, ctx, tick, grid,
                                );
                            }
                        }
                        AiState::Menacing => {
                            if Some(crate::entity_id::PcId(enemy)) != self.guarded_pc {
                                self.event_view_standard_procedure(
                                    sim, enemy, global, ctx, tick, grid,
                                );
                            }
                        }
                        AiState::Fleeing => {
                            // Ignore EVENT_VIEW while fleeing to leave the
                            // map (merry man flee) or while running back
                            // for arrow reserves.
                            if self.base.current_substate == Substate::FleeingMerryManRunToLeaveMap
                                || self.base.current_substate == Substate::FleeingMerryManLeaveMap
                                || self.base.current_substate
                                    == Substate::FleeingRunForArrowReserves
                            {
                                // ignore — committed to leaving / resupply
                            } else if self.base.current_substate == Substate::FleeingHiding
                                || self.fleeing_seen_enemy_counter < 20
                            {
                                self.fleeing_seen_enemy_counter += 1;
                                // Indoors we escalate to a building-wide
                                // alert; outdoors we kick off a directed
                                // panic away from the enemy.
                                if ctx.in_building {
                                    self.request_enemy_in_house_alert(ctx);
                                } else {
                                    let center = ctx
                                        .entity_view(enemy)
                                        .map(|v| v.position)
                                        .unwrap_or(self.base.seek_position);
                                    self.panic_from_position(
                                        center,
                                        crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                                    );
                                }
                            }
                        }
                        AiState::Attacking => {
                            // Per-substate dispatch. Do NOT fall through to
                            // a generic recovery path.
                            match self.base.current_substate {
                                Substate::AttackingReactiontimeTurning
                                | Substate::AttackingReactiontime
                                | Substate::AttackingReactiontimeRunning
                                | Substate::AttackingOverviewLookLeft
                                | Substate::AttackingOverviewLookRight
                                | Substate::AttackingTooProudToAttackOverview => {
                                    // Just track the extra enemy.
                                    // Original mlistThem is SBListUnique: the
                                    // preceding VIEW may already have rebuilt
                                    // this target into the final visible set.
                                    if !self.list_them.contains(&enemy) {
                                        self.list_them.push(enemy);
                                    }
                                }

                                Substate::AttackingArcherWaitOnArcheryPath
                                | Substate::AttackingArcherWaitOnBendPoint
                                | Substate::AttackingArcherWaitOnArcheryPathBending => {
                                    // Archer waiting on firing point —
                                    // rebuild list, re-eval elevation,
                                    // re-run BattleDecisions.
                                    self.reinitialize_them_list(ctx, tick);
                                    self.enemy_seen_below = enemy_is_below_me(
                                        ctx,
                                        tick.owner_live_position.or(Some(ctx.position)),
                                        tick.enemy_detectable_live_world_position(enemy).or_else(
                                            || {
                                                ctx.entity_view(enemy)
                                                    .map(|view| view.detection_position_world)
                                            },
                                        ),
                                    );
                                    self.battle_decisions(sim, global, ctx, tick, grid);
                                }

                                Substate::AttackingApproachingSleepingEnemy
                                | Substate::AttackingKillingSleepingEnemy => {
                                    // On seeing a new enemy while
                                    // approaching / killing a sleeping
                                    // target, pivot to standard engage
                                    // unless the sighted enemy is itself
                                    // unconscious (still not a threat).
                                    let target_unconscious = ctx
                                        .entity_view(enemy)
                                        .map(|v| v.is_unconscious)
                                        .unwrap_or(false);
                                    if !target_unconscious {
                                        self.event_view_standard_procedure(
                                            sim, enemy, global, ctx, tick, grid,
                                        );
                                    }
                                }

                                // Indoor door-fight — escalate to
                                // building-wide alert.
                                Substate::AttackingDoorFightDelay
                                | Substate::AttackingDoorFightLeaving
                                    if ctx.in_building =>
                                {
                                    self.request_enemy_in_house_alert(ctx);
                                }

                                Substate::AttackingRiderChargingGettingDistance
                                | Substate::AttackingRiderChargingReturning
                                | Substate::AttackingRiderChargingApproachingBlindly => {
                                    // Rider mid-charge sees a new enemy —
                                    // rebuild list, maybe re-target the
                                    // charge, else fall back to
                                    // BattleDecisions.
                                    self.reinitialize_them_list(ctx, tick);
                                    if !self.maybe_make_rider_attack(ctx, tick, grid) {
                                        self.battle_decisions(sim, global, ctx, tick, grid);
                                    }
                                }

                                _ => {}
                            }
                        }
                    }
                }
            }

            StimulusType::EventSeesShadow => {
                if let StimulusInfo::Position(ref pos) = stimulus.info
                    && self.base.current_state == AiState::Default
                    && !self
                        .dispatch_stimulus_to_whole_patrol(sim, stimulus, global, ctx, tick, grid)
                {
                    self.event_sees_shadow_standard_procedure(pos, ctx, tick);
                }
            }

            StimulusType::EventArrowLaunched => {
                // A shield bearer whose current substate says "I am
                // holding / advancing under a shield" slams the shield up
                // against the incoming arrow and pivots to face the
                // shooter.
                if let StimulusInfo::Human(shooter) = stimulus.info {
                    // ProtectingWithShield: bProtect = already in
                    // WAITING_SHIELD?  false : true — i.e., only re-raise
                    // if we're still mid-animation.
                    // Advancing / RunningToPhalanx: always protect.
                    let b_protect = match self.base.current_substate {
                        Substate::AttackingProtectingWithShield => ctx
                            .entity_view(self.base.me)
                            .map(|v| v.current_animation != crate::order::OrderType::WaitingShield)
                            .unwrap_or(false),
                        Substate::AttackingAdvancingWithShield
                        | Substate::AttackingRunningToPhalanx => true,
                        _ => false,
                    };

                    if b_protect {
                        use crate::element::Command;
                        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

                        // Remember the shooter.
                        self.base.primary_target = shooter;

                        self.base.stop_all();

                        // Launch RaiseShieldInstantly with
                        // ShieldDangerPoint = primary target pos.
                        let shooter_pos = ctx
                            .entity_view(shooter)
                            .map(|v| v.position)
                            .unwrap_or(self.base.seek_position);
                        let owner = self.base.owner_entity_id;
                        let mut elem =
                            SequenceElement::new_generic(1, Command::RaiseShieldInstantly, owner);
                        elem.set_property(
                            Field::ShieldDangerPoint,
                            FieldValue::Point3D {
                                x: shooter_pos.x,
                                y: shooter_pos.y,
                                z: 0.0,
                            },
                        );
                        let mut seq = Sequence::new();
                        seq.append_element(elem);
                        self.base.outbox.actor.launch_sequences.push(seq);

                        // Original immediately repeats SetStates +
                        // UpdateShield after the synchronous instant-raise
                        // launch, then Focuses the shooter. Close that actor
                        // prefix so the trailing Focus cannot overtake it at
                        // the deferred owner boundary.
                        self.base.outbox.actor.raise_shield_immediately = true;
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                &mut self.base.outbox.actor,
                            )),
                        );

                        self.base.outbox.actor.set_focus(shooter);

                        self.set_state(AiState::Attacking, Substate::AttackingProtectingWithShield);
                        self.base.launch_timer(15, ctx.frame);
                    }
                }
            }

            StimulusType::EventHear => {
                match self.base.current_state {
                    AiState::Sleeping
                    | AiState::Default
                    | AiState::Wondering
                    | AiState::Seeking => {
                        if let StimulusInfo::Noise(ref noise) = stimulus.info
                            && !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            )
                        {
                            self.event_hear_standard_procedure(sim, noise, ctx, tick);
                        }
                    }
                    _ => {} // ignore in menacing/fleeing/attacking
                }
            }

            StimulusType::EventGetArrow => {
                match self.base.current_state {
                    AiState::Sleeping
                    | AiState::Default
                    | AiState::Wondering
                    | AiState::Seeking => {
                        if let StimulusInfo::Position(ref pos) = stimulus.info
                            && !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            )
                        {
                            self.event_get_arrow_standard_procedure(sim, pos, global, ctx, tick);
                        }
                    }
                    _ => {} // ignore
                }
            }

            StimulusType::EventEnterSwordfight => {
                if let StimulusInfo::Human(enemy) = stimulus.info {
                    // The reference only asserts `!IsFriend(target)` and
                    // `IsAllowedToAttack(target)`; release builds still
                    // enter the swordfight. Do not turn those asserts
                    // into a runtime rejection, or BeginSwordFight can
                    // attach opponents while this AI stays in its old
                    // state.
                    let target_is_friend = ctx
                        .entity_view(enemy)
                        .map(|v| v.camp == ctx.camp)
                        .unwrap_or(false);
                    if target_is_friend {
                        tracing::warn!(
                            me = self.base.me,
                            enemy,
                            "EVENT_ENTER_SWORDFIGHT target is friendly; matching reference release behavior and entering anyway"
                        );
                    }
                    let allowed_to_attack = self.is_allowed_to_attack(enemy, ctx, tick);
                    if !allowed_to_attack {
                        tracing::warn!(
                            me = self.base.me,
                            enemy,
                            "EVENT_ENTER_SWORDFIGHT target is not allowed by IsAllowedToAttack; matching reference release behavior and entering anyway"
                        );
                    }
                    self.base.primary_target = enemy;
                    self.enemy_seen_below = false;
                    self.base.set_transient_emoticon(EmoticonType::XMark, 30, 0);
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.nearby_civilians_panic();
                    self.base.launch_timer(20, ctx.frame);
                }
            }

            StimulusType::EventSeesBody => {
                match self.base.current_state {
                    AiState::Sleeping
                    | AiState::Default
                    | AiState::Wondering
                    | AiState::Seeking => {
                        if let StimulusInfo::Human(body) = stimulus.info
                            && !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            )
                        {
                            self.event_sees_body_standard_procedure(body, ctx, tick, grid);
                        }
                    }
                    _ => {} // ignore in menacing/fleeing/attacking
                }
            }

            StimulusType::EventSeesObject => {
                match self.base.current_state {
                    AiState::Sleeping
                    | AiState::Default
                    | AiState::Wondering
                    | AiState::Seeking => {
                        if let StimulusInfo::Object(obj) = stimulus.info
                            && !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            )
                        {
                            self.event_sees_object_standard_procedure(obj, ctx, tick);
                        }
                    }
                    _ => {} // ignore
                }
            }

            StimulusType::CallLookThere => {
                // The sender selects eligible recipients before making this
                // synchronous call. The receiver itself has no state gate:
                // re-entrant work may change its state between selection and
                // delivery, but the Original still runs the standard body.
                if let StimulusInfo::Hint(ref hint) = stimulus.info
                    && !self
                        .dispatch_stimulus_to_whole_patrol(sim, stimulus, global, ctx, tick, grid)
                {
                    self.call_look_there_standard_procedure(&hint.seek_point, ctx, tick);
                }
            }

            StimulusType::CallTowerGuardAlert => {
                if let StimulusInfo::Hint(ref hint) = stimulus.info {
                    match self.base.current_state {
                        #[allow(clippy::collapsible_match)]
                        AiState::Default | AiState::Wondering => {
                            if !self.dispatch_stimulus_to_whole_patrol(
                                sim, stimulus, global, ctx, tick, grid,
                            ) {
                                self.call_tower_guard_alert_standard_procedure(hint, ctx, tick);
                            }
                        }
                        _ => {}
                    }
                }
            }

            StimulusType::CallTowerGuardCallsMe => {
                if let StimulusInfo::Hint(ref hint) = stimulus.info {
                    match self.base.current_state {
                        AiState::Default | AiState::Wondering => {
                            self.call_tower_guard_calls_me_standard_procedure(
                                sim, hint, global, grid, ctx, tick,
                            );
                        }
                        _ => {}
                    }
                }
            }

            StimulusType::EventGotHit => {
                // Three arms: (1) swordfighting → add opponent if
                // cross-camp & not already engaged; (2) MenacingPcInComa →
                // return-to-pc transition with NULL-opponent
                // ENTER_SWORDFIGHT sequence; (3) generic else → stop_all
                // + non-human filter + brawl-friend-in-trouble +
                // attack_enemy + SetViewStatus(EyesDieOrGetUnconscious).
                // Original checks RHElementActorHuman::IsSwordfighting(),
                // which is derived from the live opponent list.  The AI
                // substate can remain AttackingSwordfight briefly after the
                // last opponent has been removed, so it is not an equivalent
                // predicate here.
                if ctx.is_swordfighting {
                    if let StimulusInfo::Human(attacker) = stimulus.info {
                        // Only enroll if cross-camp and not already an
                        // opponent.
                        let attacker_view = ctx.entity_view(attacker).unwrap_or_else(|| {
                            panic!(
                                "soldier {} EVENT_GOTHIT requires attacker {attacker} entity view",
                                self.base.me
                            )
                        });
                        let attacker_is_friend = attacker_view.camp == ctx.camp;
                        if !attacker_is_friend {
                            let already_opponent = self
                                .find_fighter(self.base.me, tick)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "soldier {} EVENT_GOTHIT requires self fighter snapshot",
                                        self.base.me
                                    )
                                })
                                .has_as_opponent(attacker);
                            if !already_opponent {
                                self.base.outbox.actor.enter_swordfight =
                                    Some(EnterSwordfightRequest::Direct(attacker));
                            }
                        }
                    }
                } else if self.base.current_substate == Substate::MenacingPcInComa {
                    // Menacing soldier hit — pivot to
                    // ATTACKING_RETURN_TO_OTHER_PC_AFTER_MENACING, queue
                    // ENTER_SWORDFIGHT with no opponent + jump_line, face
                    // the attacker.
                    if let StimulusInfo::Human(attacker) = stimulus.info {
                        self.set_state(
                            AiState::Attacking,
                            Substate::AttackingReturnToOtherPcAfterMenacing,
                        );
                        self.base.primary_target = attacker;
                        self.base.outbox.actor.enter_swordfight =
                            Some(EnterSwordfightRequest::RaiseSword);
                        self.base.outbox.actor.enter_swordfight_jump_line = None;
                        // Original calls RHElement::SetDirection here, not
                        // RHArtificialIntelligence::Face.  The hit animation
                        // owns the gradual turn, so write only its direction
                        // goal; launching a standalone Turn is both too late
                        // and gets postponed behind RECEIVE_HIT_DAMAGE.
                        self.base.set_direction_toward_entity(attacker, ctx);
                    }
                } else {
                    // Generic effect-of-hit branch.
                    self.base.stop_all();
                    if let StimulusInfo::Human(attacker) = stimulus.info {
                        let attacker_view = ctx.entity_view(attacker);
                        let attacker_is_soldier =
                            attacker_view.map(|v| v.is_soldier()).unwrap_or(false);
                        let attacker_in_brawl = attacker_view
                            .map(|v| v.ai_substate.is_fight_for_money())
                            .unwrap_or(false);
                        if attacker_is_soldier {
                            if attacker_in_brawl {
                                // Brawl-friend hit me — capture as
                                // friend_in_trouble, transition to
                                // WonderingBrawlGotHit, clear emoticon.
                                self.base.friend_in_trouble = attacker;
                                self.set_state(AiState::Wondering, Substate::WonderingBrawlGotHit);
                                self.base.set_emoticon(EmoticonType::None);
                            }
                            // Soldier-attacker in non-brawl substate:
                            // falls through the empty switch — no
                            // primary_target / attack_enemy update; only
                            // SetViewStatus below applies.
                        } else {
                            // Non-soldier human attacker — retarget and
                            // attack.
                            self.base.primary_target = attacker;
                            self.attack_enemy(attacker, Some(&mut *global), ctx, tick, grid);
                        }
                        // SetViewStatus(EYES_DIE_OR_GET_UNCONSCIOUS)
                        // applies whenever the attacker info was human,
                        // regardless of which sub-arm fired.
                        // Keep this on the owner FIFO: in the Original this
                        // statement is the tail of EVENT_GOTHIT, after every
                        // StopAll()/AttackEnemy() actor call has completed.
                        // Close the actor prefix explicitly: AttackEnemy can
                        // reach another StopAll whose deferred Halt condolence
                        // produces Unfocus. Leaving that Halt in the ordinary
                        // actor outbox would apply Unfocus after this tail and
                        // restore LookForward.
                        if self.base.outbox.actor.has_boundary_work() {
                            self.base.outbox.reentrant.owner_work.push(
                                crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                    &mut self.base.outbox.actor,
                                )),
                            );
                        }
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::SetEyeStatus(
                                crate::element::EyeStatus::DieOrGetUnconscious,
                            ),
                        );
                    } else {
                        // Non-human stimulus info — clear primary_target.
                        self.base.primary_target = 0;
                    }
                }
            }

            StimulusType::EventApple => {
                if !self.base.current_substate.is_any_swordfight()
                    && let StimulusInfo::Position(ref pos) = stimulus.info
                {
                    self.base.stop_all();
                    self.base.seek_position = *pos;
                    self.set_state(AiState::Wondering, Substate::WonderingAppleSauceInTheVisor);
                    // Spawn a
                    // `RHTITBIT_WEAK_STUNNED` titbit at
                    // `ComputeStarsPoint` if one doesn't already exist on
                    // this NPC.  The AI can't touch the titbit manager,
                    // so we lean on `EngineInner::sync_apple_sauce_titbits`
                    // which runs every frame, scans for any NPC in
                    // `WonderingAppleSauceInTheVisor`, and calls
                    // `add_weak_stunned` — which internally runs
                    // `TitbitExists` guard + `compute_stars_point`.  The
                    // effect is same-frame (AI ticks before `sync_titbits`
                    // in `perform_hourglass_inner`).
                    // Apple hits visor, vision is restored gradually via
                    // SlowlyOpenEyes (view cone grows from 5 back to
                    // standard radius).
                    self.base.outbox.actor.slowly_open_eyes = true;
                    self.base.launch_timer(60, ctx.frame);
                }
            }

            StimulusType::EventStone => {
                match self.base.current_state {
                    AiState::Sleeping | AiState::Default | AiState::Wondering => {
                        if let StimulusInfo::Position(ref pos) = stimulus.info {
                            self.get_angry_about_apple(pos, ctx, tick);
                        }
                    }
                    _ => {} // ignore
                }
            }

            StimulusType::EventDoorCombat => {
                if let StimulusInfo::DoorCombat(ref dc) = stimulus.info {
                    self.base.primary_target = dc.adversary;
                    self.base.seek_position = dc.goal;
                    self.gather_direction = dc.direction;
                    self.set_state(AiState::Attacking, Substate::AttackingDoorFightDelay);
                    self.base.launch_timer(dc.delay as u32, ctx.frame);
                }
            }

            StimulusType::EventStop => {
                match self.base.current_state {
                    AiState::Sleeping => return false,
                    AiState::Attacking if self.base.current_substate.is_real_swordfight() => {
                        return false;
                    }
                    _ => {}
                }
                self.set_state(AiState::Seeking, Substate::SeekingGotStopEvent);
                self.base.stop_all();
                self.base.set_emoticon(EmoticonType::QuestionMark);
                // BlinkEnemy clears the seen_now/seen_last_frame flags on every
                // enemy detectable so the next detection pass treats
                // anyone still in the cone as a "first-seen" edge and
                // re-issues EVENT_VIEW.  Without this, an NPC that was
                // already tracking the PC before the EVENT_STOP would
                // stay in SeekingGotStopEvent forever (the visibility
                // edge-trigger never fires) once the stop timer elapses.
                self.base.outbox.actor.blink_all_enemies = true;
                self.base.launch_timer(100, ctx.frame);
            }

            StimulusType::EventSeesFriendInTrouble => {
                if let StimulusInfo::Combat(ref combat) = stimulus.info
                    && self.answer_question(Question::ShallIHelpFriendInTrouble, ctx)
                {
                    self.base.friend_in_trouble = combat.actor_npc;
                    self.base.seek_position = combat.enemy_position;
                    self.current_task_priority = task_priority::FRIEND_IN_TROUBLE;
                    self.set_state(AiState::Seeking, Substate::SeekingCombatAlertReactiontime);
                    self.react(
                        parameters_ai::AI_MAX_FRIENDINTROUBLE_REACTIONTIME as u16,
                        ctx,
                        tick,
                    );
                }
            }

            StimulusType::EventPcShotAtMe => {
                if let StimulusInfo::Human(enemy) = stimulus.info {
                    self.event_view_standard_procedure(sim, enemy, global, ctx, tick, grid);
                }
            }

            _ => {
                tracing::trace!(
                    "EnemyAi::think_alerting_event: unhandled {:?} in {:?}",
                    stimulus_type,
                    self.base.current_substate,
                );
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Standard procedures (called from ThinkAlertingEvent)
    // -----------------------------------------------------------------------

    /// React to seeing an enemy. Port of `EventViewStandardProcedure`.
    fn event_view_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        enemy: HumanHandle,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        tracing::trace!(
            me = self.base.me,
            enemy,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            primary_target = self.base.primary_target,
            frame = ctx.frame,
            "event_view_standard_procedure: ENTRY"
        );
        if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
            return;
        }
        self.current_task_priority = self.new_task_priority;

        // Royalist-camp early returns. These guards are NOT hoisted into
        // the engine-side dispatcher (which only filters on state), so
        // they must live here to avoid green soldiers chasing
        // already-tied / already-guarded targets and archers on
        // unreachable wall-tops.
        let enemy_view = ctx.entity_view(enemy);
        if ctx.camp == crate::element::Camp::Royalists
            && let Some(v) = enemy_view
            && (v.is_unconscious || v.posture == crate::element::Posture::Tied || v.is_carried)
        {
            return;
        }
        if let Some(v) = enemy_view
            && v.is_pc
            && v.guard.is_some()
        {
            return;
        }
        if ctx.camp == crate::element::Camp::Royalists
            && let Some(v) = enemy_view
            && v.elevation > ctx.elevation + 100.0
            && v.is_soldier()
            && !v.is_archer
        {
            return;
        }

        self.base.outbox.detection.mark_alerted = true;
        self.base.frame_when_enemy_detected = ctx.frame;
        // Only meaningful for archers, who use the flag to switch to
        // bow-down posture.
        self.enemy_seen_below = enemy_is_below_me(
            ctx,
            tick.owner_live_position.or(Some(ctx.position)),
            tick.enemy_detectable_live_world_position(enemy)
                .or_else(|| enemy_view.map(|view| view.detection_position_world)),
        );

        // Forget old object of desire
        if self.base.object_of_desire != 0 {
            self.base.forgotten_objects.push(self.base.object_of_desire);
            self.base.object_of_desire = 0;
        }

        // Resolve a *fresh* enemy position once and use it for the
        // recon report, the friend-alert broadcast, and the run-near
        // destination — the reference re-reads `Position(pEnemy)`
        // literally at each call site rather than using the stale
        // `mposSeekPosition`.
        let enemy_pos = tick
            .enemy_detectable_position(enemy)
            .or_else(|| enemy_view.map(|v| v.position))
            .unwrap_or(self.base.seek_position);

        // Update recon report.
        self.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, enemy_pos);

        // Soldier inside a building must escalate to a building-wide alarm
        // before doing anything else.
        if ctx.in_building {
            self.request_enemy_in_house_alert(ctx);
            return;
        }

        self.reinitialize_them_list(ctx, tick);

        // Recognize lost enemy
        if self.pc_missed && self.missed_pc == enemy {
            self.pc_missed = false;
        }

        // HeyFolksLookThere(enemy_pos, VIEW_LOOK_THERE_RADIUS). It runs before
        // the state transition below, and the called friends think inside it,
        // so the tail has to wait for them.
        if self.hey_folks_look_there(
            &enemy_pos,
            100,
            LookThereContinuation::EventView { enemy, enemy_pos },
            ctx,
        ) {
            return;
        }
        self.event_view_after_look_there(sim, enemy, enemy_pos, global, ctx, tick, grid);
    }

    pub(super) fn event_view_after_look_there(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        enemy: HumanHandle,
        enemy_pos: Position,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Already sprinting? Stay in MovingFast, just commit the target
        // and re-issue the run-to. Skips the StopAll/Say path entirely so
        // the sprint animation chains straight into the engage.
        if ctx.self_action_state == crate::element::ActionState::MovingFast {
            self.set_state(AiState::Attacking, Substate::AttackingReactiontimeRunning);
            self.base.primary_target = enemy;
            self.base.outbox.actor.set_focus(enemy);
            self.reinitialize_them_list(ctx, tick);
            // GoNear(Position(pEnemy), Distance/3, GOTO_RUN)
            let owner_live_position = tick.owner_live_position.unwrap_or_else(|| {
                panic!(
                    "moving-fast EVENT_VIEW for {} requires the owner's literal live position",
                    self.base.me
                )
            });
            let enemy_view = ctx
                .entity_view(enemy)
                .unwrap_or_else(|| panic!("EVENT_VIEW target {enemy} requires a live entity view"));
            let enemy_live_position = Position {
                x: enemy_view.detection_position.x,
                y: enemy_view.detection_position.y,
                sector: enemy_view.position.sector,
                level: enemy_view.position.level,
            };
            // `Distance(pEnemy)` subtracts the actors' literal 3D
            // `GetPosition()` points, stretches world Y by
            // INVERSE_ASPECT_RATIO, then takes the Euclidean norm. The
            // positions carried here are map-space, so recover world Y with
            // each actor's elevation before dividing by three.
            let distance = ai_square_distance(
                &enemy_live_position,
                enemy_view.detection_position_world.z,
                &owner_live_position,
                ctx.elevation,
            )
            .sqrt();
            let radius = (distance / 3.0).max(0.0) as i32;
            self.base
                .go_near(enemy_pos, radius, crate::ai::GotoFlags::RUN, ctx);
            self.base.launch_timer(10, ctx.frame);
            tracing::trace!(
                me = self.base.me,
                state = ?self.base.current_state,
                substate = ?self.base.current_substate,
                primary_target = self.base.primary_target,
                "event_view_standard_procedure: EXIT (moving-fast)"
            );
            return;
        }

        // Stop and engage
        self.base.stop_all();
        self.base.say(Remark::SeesEnemy);

        self.base.primary_target = enemy;
        self.base.outbox.actor.set_focus(enemy);
        self.reinitialize_them_list(ctx, tick);
        // EventViewStandardProcedure
        // does NOT set `EMOTICON_X_MARK` here — the red `!` only
        // appears when `AttackEnemy` fires (line 8171) after the
        // reaction-time window closes.

        // Three-branch dispatch based on distance and below-flag.
        // `MaxNormDistance(pEnemy)` deliberately bypasses AI `Position()` and
        // subtracts the actors' literal `GetPosition()` values. During a door
        // pass, `enemy_pos` and `ctx.position` are instead forecast onto the
        // destination gate side. Keep those forecast positions for the report,
        // alert, focus, and Face calls above/below, but use the raw element
        // positions for this gate exactly as the Original does.
        let enemy_view = ctx
            .entity_view(enemy)
            .unwrap_or_else(|| panic!("EVENT_VIEW target {enemy} requires a live entity view"));
        let enemy_live_position = Position {
            x: enemy_view.detection_position.x,
            y: enemy_view.detection_position.y,
            sector: enemy_view.position.sector,
            level: enemy_view.position.level,
        };
        let owner_live_position = tick.owner_live_position.unwrap_or_else(|| {
            panic!(
                "EVENT_VIEW for {} requires the owner's literal live position",
                self.base.me
            )
        });
        let max_norm_dist = ai_max_norm_distance(
            &enemy_live_position,
            enemy_view.detection_position_world.z,
            &owner_live_position,
            ctx.elevation,
        );
        if max_norm_dist < 50.0 {
            // Enemy very near — skip the turn and dispatch BattleDecisions
            // immediately. `IAmInTrouble` is called only on this branch
            // (the broader sightings stay quiet).
            self.set_state(AiState::Attacking, Substate::AttackingReactiontime);
            self.i_am_in_trouble(enemy);
            self.battle_decisions(sim, global, ctx, tick, grid);
        } else if self.enemy_seen_below {
            // Archer saw enemy from a wall — no turn, just a short 5-tick
            // reaction to aim the bow.
            self.set_state(AiState::Attacking, Substate::AttackingReactiontime);
            self.base.launch_timer(5, ctx.frame);
        } else {
            // Standard case — turn towards enemy with a 20-tick
            // LaunchTimer as the upper bound for the turn animation.
            // `process_turn_orders` handles the snap + anim booking and
            // the live actor coordinator's `tick_actor_animation_for` fires
            // `EventDone` when the animation completes; whichever fires first
            // wins.
            self.set_state(AiState::Attacking, Substate::AttackingReactiontimeTurning);
            // Original explicitly calls Face(pEnemy, true) here: the alert
            // reaction uses TurnFast, including when the sequence is deferred
            // behind an attentive-mode transition.
            self.base.face_entity_fast(enemy, ctx);
            self.base.launch_timer(20, ctx.frame);
        }
        tracing::trace!(
            me = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            primary_target = self.base.primary_target,
            timer = self.base.when_does_timer_ring,
            "event_view_standard_procedure: EXIT"
        );
    }

    /// React to hearing a noise. Port of `EventHearStandardProcedure`.
    fn event_hear_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        noise: &Noise,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
            return;
        }
        self.current_task_priority = self.new_task_priority;

        if self.base.object_of_desire != 0 {
            self.base.forgotten_objects.push(self.base.object_of_desire);
            self.base.object_of_desire = 0;
        }

        // EventHearStandardProcedure passes `noise.posOrigin` to the
        // RHposition overload of Face. Preserve its sector projection when
        // available; replay-normalized sector-less origins fall back to the
        // separately recorded RHnoise elevation instead of fake ground zero.
        match noise.noise_type {
            NoiseType::Pfiiit => {
                // Whistling.
                //
                // If `Q_SHALL_I_LOOK_WHISTLE` is false (low whistle stat /
                // wrong rank), just glance toward the noise briefly —
                // SeekingJustWatching with a FirstLook timer. Early-return
                // before touching the reconnaissance report. An earlier
                // port collapsed this branch into the general path, which
                // caused low-attention guards to fully investigate every
                // whistle instead of just peeking.
                if !self.answer_question(Question::ShallILookWhistle, ctx) {
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.set_state(AiState::Seeking, Substate::SeekingJustWatching);
                    self.base.seek_position = noise.origin;
                    self.base.stop_all();
                    if self.base.current_state != AiState::Sleeping {
                        self.base.face_noise_origin_with_ctx(noise, ctx);
                    }
                    self.base.say(Remark::HearsNoise);
                    self.base
                        .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
                    return;
                }

                // Noise is not ignored — break a running macro.
                self.base.stop_all();
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, noise.origin);
                self.base.seek_position = noise.origin;

                if self.base.current_state == AiState::Seeking
                    && self.get_rank() != ProfileRank::Officer
                {
                    // Soldier already seeking → go directly to the noise
                    // (no emoticon set here; the soldier is already in
                    // the middle of a seek).
                    self.set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                    self.base.say(Remark::HearsNoise);
                    self.base.face_noise_origin_with_ctx(noise, ctx);
                    self.base.launch_timer(1, ctx.frame);
                } else {
                    // Idle / officer → curious-react into the wondering
                    // state.
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.base.say(Remark::HearsNoise);
                    self.set_state(AiState::Wondering, Substate::WonderingHeardWhistling);
                    self.react(
                        parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16,
                        ctx,
                        tick,
                    );
                }
            }

            NoiseType::Heeelp | NoiseType::TapTapTap | NoiseType::Aaargh | NoiseType::ZingZing => {
                // Important noises — investigate.
                // HEEELP has extra ignore conditions — riders ignore help
                // cries, and NPCs mid-JustWatching finish their look
                // before reacting.
                if noise.noise_type == NoiseType::Heeelp {
                    if ctx.self_is_rider {
                        return;
                    }
                    if self.base.current_substate == Substate::SeekingJustWatching {
                        return;
                    }
                }
                if self.base.current_substate == Substate::SeekingHeardstepsPreReactiontime
                    || self.base.current_substate == Substate::SeekingHeardstepsReactiontime
                    || self.base.current_substate.is_take_money()
                    || self.base.current_substate.is_fight_for_money()
                {
                    return; // ignore
                }

                self.base.stop_all();
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, noise.origin);
                self.base.seek_position = noise.origin;

                if self.base.current_state == AiState::Seeking
                    && self.base.current_substate != Substate::SeekingGotStopEvent
                    && self.get_rank() != ProfileRank::Officer
                {
                    self.set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                    if noise.noise_type != NoiseType::Aaargh {
                        self.base.say(Remark::HearsNoise);
                    }
                    self.base.face_noise_origin_with_ctx(noise, ctx);
                    self.base.launch_timer(1, ctx.frame);
                } else {
                    if noise.noise_type != NoiseType::Aaargh {
                        self.base.say(Remark::HearsNoise);
                    }
                    self.set_state(AiState::Seeking, Substate::SeekingHeardstepsPreReactiontime);
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    if self.base.current_state == AiState::Default {
                        self.react(parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16, ctx, tick);
                    } else {
                        self.base.launch_timer(1, ctx.frame);
                    }
                }
            }

            NoiseType::Bonk | NoiseType::Zonk | NoiseType::Pling
                if self.base.current_state == AiState::Default =>
            {
                self.base.stop_all();
                if noise.noise_type == NoiseType::Zonk {
                    self.base.say(Remark::Arrow);
                }
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.set_state(AiState::Wondering, Substate::WonderingWatching);
                self.base.seek_position = noise.origin;
                self.base.face_noise_origin_with_ctx(noise, ctx);
                self.base.launch_timer(50, ctx.frame);
            }

            NoiseType::Logs | NoiseType::Drawbridge
                if self.base.current_state == AiState::Default =>
            {
                self.base.stop_all();
                self.set_state(AiState::Wondering, Substate::WonderingWatching);
                self.base.seek_position = noise.origin;
                self.base.face_noise_origin_with_ctx(noise, ctx);
                self.base.launch_timer(
                    70 + crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::SoldierNoiseCooldown,
                        0..60,
                    ),
                    ctx.frame,
                );
            }

            _ => {}
        }
    }

    /// React to seeing a body. Port of `EventSeesBodyStandardProcedure`.
    fn event_sees_body_standard_procedure(
        &mut self,
        body: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // bHeyThisIsCharly captures whether this body is the soldier we
        // were tasked to find via a MissedCharly recon report — used
        // twice below to fire the unalert-cascade on the seeker network.
        let body_view = ctx.entity_view(body);
        let body_pos = body_view
            .map(|v| v.position)
            .unwrap_or(self.base.seek_position);
        let b_hey_this_is_charly = self.base.current_state == AiState::Seeking
            && self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly
            && self.base.my_reconnaissance_report.charly == body;

        self.base.my_reconnaissance_report.add_seen_body(body);
        // Update(REPORT_BODY, Position(pBody)) — must use the body's
        // position, not the stale `seek_position`.
        self.base
            .my_reconnaissance_report
            .update(ReportType::Body, body_pos);

        // Dead NPC corpse → push onto missed_in_action so officer-report
        // and recon downstream know a friend died.
        if let Some(v) = body_view
            && v.is_dead
            && (v.kind == crate::ai_entity_view::EntityKind::Soldier
                || v.kind == crate::ai_entity_view::EntityKind::Civilian)
        {
            self.base
                .missed_in_action
                .push(body as crate::ai::NpcHandle);
        }

        if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
            return;
        }
        self.current_task_priority = self.new_task_priority;

        if self.base.object_of_desire != 0 {
            self.base.forgotten_objects.push(self.base.object_of_desire);
            self.base.object_of_desire = 0;
        }

        // Already on the way to a body? queue for later.
        match self.base.current_substate {
            Substate::SeekingBodyReactiontime
            | Substate::SeekingBody
            | Substate::SeekingNet
            | Substate::SeekingBodyLookingDeadBody
            | Substate::SeekingBodyAwakeningSleeperr => {
                if body != self.base.detected_body {
                    self.other_bodies_to_examine.push(body);
                }
                return;
            }
            // Mid-seek-of-charly arms (SEEKPOINT / CHARLY /
            // AMBUSH_LEFT/RIGHT / CHECKING_AMBUSH). If this body *is*
            // charly, fire the unalert cascade; then detour into the
            // body-examination flow and return.
            Substate::SeekingSeekpoint
            | Substate::SeekingCharly
            | Substate::SeekingSeekpointPassedAmbushPointLeft
            | Substate::SeekingSeekpointPassedAmbushPointRight
            | Substate::SeekingSeekpointCheckingAmbushPoint => {
                if b_hey_this_is_charly {
                    // The body we're seeing is charly — broadcast the
                    // unalert.
                    self.base.outbox.actor.queue_unalert_near_charly_seekers(
                        CharlySeekerTarget::Npc(body),
                        self.base.antagonist,
                    );
                }
                self.run_to_examine_body(body, ctx, tick, grid);
                return;
            }
            _ => {}
        }

        // Stuck-under-net → different remark.
        let stuck = body_view.map(|v| v.stuck_under_net).unwrap_or(false);
        if stuck {
            self.base.say(Remark::SeesFriendUnderNet);
        } else {
            self.base.say(Remark::SeesBody);
        }
        // HeyFolksLookThere with default radius.
        if self.hey_folks_look_there(
            &body_pos,
            100,
            LookThereContinuation::EventSeesBody {
                body,
                body_pos,
                is_charly: b_hey_this_is_charly,
            },
            ctx,
        ) {
            return;
        }
        self.event_sees_body_after_look_there(body, body_pos, b_hey_this_is_charly, ctx, tick);
    }

    pub(super) fn event_sees_body_after_look_there(
        &mut self,
        body: HumanHandle,
        body_pos: Position,
        b_hey_this_is_charly: bool,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.seen_dead_body = false;

        self.base.stop_all();
        // Remember the body and its position.
        self.base.seek_position = body_pos;
        self.base.detected_body = body;
        self.base.outbox.actor.set_focus(body);

        self.set_state(AiState::Seeking, Substate::SeekingBodyReactiontime);

        // Turn to look at the body.
        // Original uses `Face(RHposition)`: preserve its 3D projection and
        // its same-direction Waiting/Bored short-circuit.
        self.base.face_position_3d_with_ctx(body_pos, ctx);
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // Post-SetState charly check fires the unalert cascade for
        // non-mid-seek discoveries too. The body we just saw IS charly,
        // so the sweep target is the body handle.
        if b_hey_this_is_charly {
            self.base.outbox.actor.queue_unalert_near_charly_seekers(
                CharlySeekerTarget::Npc(body),
                self.base.antagonist,
            );
        }
        self.react(
            parameters_ai::AI_MAX_DEADBODY_REACTIONTIME as u16,
            ctx,
            tick,
        );
    }

    /// React to seeing an arrow impact. Port of `EventGetArrowStandardProcedure`.
    fn event_get_arrow_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pos: &Position,
        global: &AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.current_task_priority = task_priority::ENEMY;

        if self.base.object_of_desire != 0 {
            self.base.forgotten_objects.push(self.base.object_of_desire);
            self.base.object_of_desire = 0;
        }

        self.base.stop_all();
        self.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, *pos);

        if self.base.current_state == AiState::Seeking && self.get_rank() != ProfileRank::Officer {
            self.set_state(AiState::Seeking, Substate::SeekingArrowReactiontime);
            self.base.seek_position = *pos;
            // Snap onto a nearby seek point (0.3 of me→origin, no
            // absolute).
            global.set_pos_on_near_seek_point(
                sim,
                ctx.position,
                &mut self.base.seek_position,
                0.3,
                0,
            );
            let seek = self.base.seek_position;
            self.base.face_position_3d_with_ctx(seek, ctx);
            self.base.launch_timer(1, ctx.frame);
        } else {
            // Switch on rank between soldier/knight (go investigate) and
            // officer (just watch from current position).
            self.base.set_emoticon(EmoticonType::QuestionMark);
            let substate = if self.get_rank() == ProfileRank::Officer {
                Substate::SeekingArrowJustWatching
            } else {
                Substate::SeekingArrowReactiontime
            };
            self.set_state(AiState::Seeking, substate);
            self.base.seek_position = *pos;
            // Both arms snap the seek target onto a nearby seek point.
            global.set_pos_on_near_seek_point(
                sim,
                ctx.position,
                &mut self.base.seek_position,
                0.3,
                0,
            );
            let seek = self.base.seek_position;
            self.base.face_position_3d_with_ctx(seek, ctx);
            // Focus on the interesting object — locks the eye-tracking
            // cone onto the arrow's interesting object so the detection
            // cone narrows along the threat axis.
            // `Focus(mpInterestingObject)` is unconditional in the Original.
            // A null interesting object is meaningful: `Focus(NULL)` calls
            // `Unfocus()` and clears a stale point-focus left by an earlier
            // CALL_LOOKTHERE in the patrol's synchronous arrow broadcast.
            self.base
                .outbox
                .actor
                .set_focus(self.base.interesting_object);
            if !self.hey_folks_look_there(pos, 200, LookThereContinuation::EventGetArrow, ctx) {
                self.event_get_arrow_after_look_there(ctx, tick);
            }
        }
    }

    pub(super) fn event_get_arrow_after_look_there(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if self.get_rank() == ProfileRank::Officer {
            // Officer just watches with a fixed timer.
            self.base
                .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
        } else {
            self.react(
                parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16 + 50,
                ctx,
                tick,
            );
        }
    }

    fn event_sees_shadow_standard_procedure(
        &mut self,
        pos: &Position,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        // Ignore shadow when in building or leaning out.
        if ctx.in_building || ctx.posture == crate::element::Posture::LeaningOut {
            return;
        }

        self.base.stop_all();
        self.set_state(AiState::Default, Substate::DefaultLookingShadow);
        // A shadow raises only the music-side alert. The view remains green,
        // so ordinary PC detection keeps its two-frame refresh cadence.
        // Original: SetAlertStatus(ALERT_YELLOW, ALERT_ONLY_MUSIC).
        self.set_alert_status_with_flags(AlertLevel::Yellow, crate::ai::AlertFlags::ONLY_MUSIC);
        self.base.face_position_3d_with_ctx(*pos, ctx);
        self.base.launch_timer(10, ctx.frame);
    }

    fn event_sees_object_standard_procedure(
        &mut self,
        obj: ObjectHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Outer switch on object type. Ale and money (purse/coin) take
        // very different paths; everything else is a no-op for the AI.
        use crate::element_kinds::ObjectType;
        let obj_type = ctx
            .entity_view(obj)
            .map(|v| v.object_type)
            .unwrap_or(ObjectType::None);

        match obj_type {
            ObjectType::Purse | ObjectType::Coin => {
                // Already committed to a money/brawl
                // substate?  Just queue the sighting onto
                // `other_seen_money` and skip the reactiontime reset.
                if self.base.current_substate.is_take_money()
                    || self.base.current_substate.is_fight_for_money()
                    || matches!(
                        self.base.current_substate,
                        Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl
                            | Substate::WonderingApproachingBrawlVictim
                            | Substate::WonderingAwakenBrawlVictim
                    )
                {
                    self.other_seen_money.push(obj);
                    return;
                }

                // Default arm.
                self.base.stop_all();
                self.base.say(Remark::SeesObject);
                self.base.interesting_object = obj;
                if let Some(view) = ctx.entity_view(obj) {
                    self.base.face_position_at_elevation_with_ctx(
                        view.position,
                        f32::from(view.elevation as u16),
                        ctx,
                    );
                }
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.set_state(AiState::Wondering, Substate::WonderingMoneyReactiontime);
                self.base.outbox.actor.set_focus(obj);
                if self.get_rank() == ProfileRank::Officer {
                    self.base.launch_timer(60, ctx.frame);
                } else {
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            ObjectType::Ale => {
                // Already committed to an ale-taking substate? Queue and
                // skip.
                if self.base.current_substate.is_take_ale() {
                    self.other_seen_ale.push(obj);
                    return;
                }

                // Default arm — note `BreakMacro` (preserves a running
                // sequence for resume) instead of the harder `StopAll`,
                // and `React(AI_FIRST_LOOK_TIME)` instead of the
                // rank-dependent fixed-tick LaunchTimer.
                self.base.break_macro();
                self.base.say(Remark::SeesObject);
                if let Some(view) = ctx.entity_view(obj) {
                    // Keep the position carried by the live object pointer.
                    // The bottle may become inactive while React's timer is
                    // pending, but Original can still read Position(pointer)
                    // when that timer fires.
                    self.base.seek_position = view.position;
                    self.base.face_position_at_elevation_with_ctx(
                        view.position,
                        f32::from(view.elevation as u16),
                        ctx,
                    );
                }
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.base.interesting_object = obj;
                self.base.outbox.actor.set_focus(obj);
                self.set_state(AiState::Wondering, Substate::WonderingAleReactiontime);
                self.react(parameters_ai::AI_FIRST_LOOK_TIME as u16, ctx, tick);
            }

            // Everything else falls through silently.
            _ => {}
        }
    }

    fn call_look_there_standard_procedure(
        &mut self,
        pos: &Position,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        if !self.is_merry_man_forest(ctx) {
            self.base
                .set_transient_emoticon(EmoticonType::QuestionMark, 10, 0);
        }
        self.base.stop_all();
        self.set_state(AiState::Wondering, Substate::WonderingWatching);
        self.base.seek_position = *pos;
        // Focus on the hint position — engage `EYES_STARE` with the
        // narrow stare cone so subsequent detection ticks cast a narrow
        // stare rather than the default look-forward cone.
        self.base.outbox.actor.set_focus_point(*pos);
        // Original `Face(RHposition)` reaches `FaceTo`, which completes
        // synchronously when an idle actor already faces the requested
        // sector. Keep the context-aware short-circuit here instead of
        // registering a redundant one-frame TURN sequence.
        self.base.face_position_3d_with_ctx(*pos, ctx);
        self.base.launch_timer(100, ctx.frame);
    }

    fn call_tower_guard_alert_standard_procedure(
        &mut self,
        hint: &Hint,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        self.base
            .set_transient_emoticon(EmoticonType::QuestionMark, 10, 0);
        self.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);

        if self.get_rank() == ProfileRank::Knight {
            self.set_state(AiState::Seeking, Substate::SeekingKnightWatchingTowerGuard);
        } else {
            self.set_state(AiState::Wondering, Substate::WonderingWatchingTowerGuard);
        }
        self.base.seek_position = hint.seek_point;
        // Focus on the reported point — engages `EYES_STARE`. Without
        // this the alerted soldier sweeps a default-angle cone and may
        // miss the enemy at the edge of the stare-cone.
        self.base.outbox.actor.set_focus_point(hint.seek_point);
        self.base.face_entity(hint.who_tells_me, ctx);
        self.base.launch_timer(100, ctx.frame);
    }

    fn call_tower_guard_calls_me_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        hint: &Hint,
        global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.base.seek_position = hint.seek_point;
        self.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);

        match self.get_rank() {
            ProfileRank::Soldier => {
                let returns_to_instructed_group =
                    self.alert_officer_returns_to_instructed_group(tick);
                let alerted = self.alert_officer(sim, self.base.seek_position, 0, ctx, tick);
                if alerted && !returns_to_instructed_group {
                    // Original AlertOfficer constructs the GoNear route and
                    // consumes mbCouldntReachpoint before returning, even
                    // though this caller ignores its bool result. Close the
                    // actor prefix first, then clear only that route failure
                    // at the typed owner tail.
                    self.base.outbox.reentrant.owner_work.push(
                        crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                            &mut self.base.outbox.actor,
                        )),
                    );
                    self.base
                        .outbox
                        .reentrant
                        .tower_guard_alert_officer_completion_pending = true;
                    self.base
                        .outbox
                        .reentrant
                        .owner_work
                        .push(crate::ai::AiOwnerWork::ConsumeTowerGuardAlertOfficerRouteFailure);
                }
                self.current_task_priority = task_priority::ALERT_IGNORE_ENEMY;
            }
            ProfileRank::Officer => {
                self.alert_soldiers(
                    self.base.seek_position,
                    0,
                    global,
                    grid,
                    ctx,
                    tick,
                    AlertSoldiersFailureContinuation::None,
                );
            }
            ProfileRank::Knight => unreachable!(
                "RANK_KNIGHT is never returned by CanCallThisSoldier for tower-guard call-me dispatch"
            ),
            ProfileRank::None => {}
        }
    }

    fn call_combat_alert_standard_procedure(
        &mut self,
        pos: &Position,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.base
            .set_transient_emoticon(EmoticonType::QuestionMark, 10, 0);
        self.set_state(AiState::Seeking, Substate::SeekingCombatAlertReactiontime);
        self.base.seek_position = *pos;
        // Focus on the position — engages `EYES_STARE` before facing it.
        self.base.outbox.actor.set_focus_point(*pos);
        self.base.face_position_3d_with_ctx(*pos, ctx);
        self.react(
            parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16,
            ctx,
            tick,
        );
    }

    /// React to an apple (or stone) strike by snapping to
    /// `WonderingAppleReactiontime` and launching the reaction timer.
    pub(super) fn get_angry_about_apple(
        &mut self,
        pos: &Position,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        // Priority gate — ignore if new task priority is lower than what
        // we're already doing.
        if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
            return;
        }
        // Commit the new priority.
        self.current_task_priority = self.new_task_priority;

        // Forget any pending desired object.
        if self.base.object_of_desire != 0 {
            self.base.forgotten_objects.push(self.base.object_of_desire);
            self.base.object_of_desire = 0;
        }

        self.base.stop_all();
        self.base.seek_position = *pos;
        self.set_state(AiState::Wondering, Substate::WonderingAppleReactiontime);

        // VIP vs soldier remark.
        let remark = if self.is_vip {
            Remark::VipAppleNo
        } else {
            Remark::HitByApple
        };
        self.base.say(remark);

        self.base.face_position_3d_with_ctx(*pos, ctx);
        self.base.set_emoticon(EmoticonType::QuestionMark);
        self.base
            .launch_timer(combat::APPLE_REACTIONTIME as u32, ctx.frame);
    }

    // -----------------------------------------------------------------------
    // CouldntReachpointEmergencyRoutine — per-state fallback when the
    // substate-specific EventCouldntReachPoint arms in
    // `think_unexpected_event` don't cover the current substate.
    // -----------------------------------------------------------------------

    pub fn couldnt_reachpoint_emergency_routine(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        use crate::element::Posture;
        // IsVeryVeryBusy preamble — lock AI BUSY, mark was_busy, re-fire
        // EVENT_COULDNT_REACHPOINT once the command/posture clears. The
        // engine supplies the live sequence-manager result through
        // `in_uninterruptible_command`; posture covers the actor-local
        // Flying/OnLadder/OnWall branches of the same Original predicate.
        if ctx.in_uninterruptible_command
            || matches!(
                ctx.posture,
                Posture::Flying | Posture::OnLadder | Posture::OnWall,
            )
        {
            self.base.non_script_lock(crate::ai::AiLockFlags::BUSY);
            self.base.was_busy = true;
            self.base
                .outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventCouldntReachPoint.into());
            return;
        }

        match self.base.current_state {
            // Sleeping / default / wondering / menacing / fleeing → return
            // to duty.
            AiState::Sleeping
            | AiState::Default
            | AiState::Wondering
            | AiState::Menacing
            | AiState::Fleeing => {
                self.return_to_duty(sim, DutyFlags::BECAUSE_COULDNT_REACHPOINT, ctx, tick);
            }
            // Dead-body sweep around the actor.
            AiState::Seeking => {
                if Self::seek_area_phase6_caller_debug_enabled()
                    && Self::seek_area_phase6_caller_debug_matches(
                        ctx.frame,
                        ctx.original_creation_order,
                    )
                {
                    eprintln!(
                        "SEEKAREA_CALLER {{\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"caller\":\"couldnt_reach_emergency\",\"stimulus\":\"event_couldnt_reach_point\"}}",
                        ctx.frame,
                        self.base.me,
                        ctx.original_creation_order.expect(
                            "phase6 caller diagnostic matched an owner without creation order"
                        ),
                    );
                }
                self.seek_area(
                    sim,
                    ctx.position,
                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::empty(),
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            // Stay in combat — swordfighters drop back into the
            // swordfight substate with a brief timer; everyone else
            // re-picks a target via GetBattleOverview.
            AiState::Attacking => {
                if ctx.is_swordfighting {
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.get_battle_overview(0, ctx, tick);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind};
    use crate::element::{Camp, Posture};
    use crate::element_kinds::ObjectType;
    use crate::order::OrderType;

    fn object_view(object_type: ObjectType) -> AiEntityView {
        AiEntityView {
            original_creation_order: 41,
            position: Position {
                x: 10.0,
                y: 20.0,
                sector: None,
                level: 0,
            },
            detection_position: crate::coordinates::MapPoint::new(10.0, 20.0),
            detection_position_world: crate::coordinates::WorldPoint3D::new(10.0, 20.0, 0.0),
            direction: 0,
            posture: Posture::Upright,
            camp: Camp::default(),
            is_pc: false,
            is_robin: false,
            is_vip: false,
            is_beggar: false,
            is_child: false,
            kind: EntityKind::Bonus,
            is_tower_guard: false,
            is_swordfighting: false,
            is_able_to_fight: false,
            active: true,
            is_unconscious: false,
            action_state: crate::element::ActionState::Waiting,
            is_moving_map: false,
            passing_door: false,
            obstacle_idx: None,
            in_building: false,
            building_sector: None,
            script_locked: false,
            forecasted_destination: crate::ai::PreparedForecastDestination::fixed(
                Position::default(),
                0,
            ),
            ai_state: AiState::Default,
            ai_substate: Substate::DefaultOnPost,
            current_animation: OrderType::WaitingUprightBored,
            elevation: 0.0,
            object_type,
            is_dead: false,
            is_carried: false,
            is_archer: false,
            is_rider: false,
            stuck_under_net: false,
            covering_nets: Vec::new(),
            in_coma: false,
            guard: None,
            has_patrol_path: false,
            initial_position: Position::default(),
            number_of_arrows: 0,
            rank: ProfileRank::None,
            reported_to_officer: false,
            looted_after_money_fight: false,
            current_money: 0,
            macro_in_progress: false,
            path_current_waypoint_index: 0,
            path_last_waypoint_index: 0,
            path_forward_movement: true,
            patrol_hiking_path_index: None,
            interesting_object: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
        }
    }

    #[test]
    fn failed_fleeing_panic_move_uses_panic_seek_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(68);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingPanic;
        ai.base.lasting_panic_runs = 7;
        ai.base.set_alert_status(crate::ai::AlertLevel::Red);

        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Red);
        assert!(ai.base.outbox.actor.panic_seek_fallback);
    }

    #[test]
    fn event_view_uses_owner_boundary_position_instead_of_stale_live_map() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let mut enemy_view = object_view(ObjectType::None);
        enemy_view.kind = EntityKind::Pc;
        enemy_view.is_pc = true;
        enemy_view.position = Position {
            x: 10.0,
            y: 0.0,
            sector: None,
            level: 0,
        };
        enemy_view.detection_position = crate::coordinates::MapPoint::new(100.0, 0.0);
        enemy_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);
        let mut views = AiEntityViewMap::new();
        views.insert(12, enemy_view);
        let ctx = AiContext {
            position: Position::default(),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(ctx.position);
        tick.enemy_detectable_positions.push((
            12,
            Position {
                x: 100.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
        ));
        assert!(tick.enemy_detectable_live_world_positions.is_empty());

        ai.event_view_standard_procedure(
            &sim,
            12,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert!(
            !ai.enemy_seen_below,
            "non-optical dispatch must use the concrete entity-view geometry when no live detectable list was prepared"
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingReactiontimeTurning
        );
    }

    #[test]
    fn moving_fast_event_view_distance_uses_literal_owner_position_during_door_pass() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(134);
        let enemy_position = Position {
            x: 1648.9281,
            y: 1804.8717,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };
        let mut enemy_view = object_view(ObjectType::None);
        enemy_view.kind = EntityKind::Pc;
        enemy_view.is_pc = true;
        enemy_view.position = enemy_position;
        enemy_view.detection_position =
            crate::coordinates::MapPoint::new(enemy_position.x, enemy_position.y);
        enemy_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(enemy_position.x, enemy_position.y, 0.0);
        let mut views = AiEntityViewMap::new();
        views.insert(342, enemy_view);
        let ctx = AiContext {
            // AI Position(owner) is already forecast onto the selected
            // door's far side, but Original Distance(enemy) directly reads
            // the still-interpolating element position instead.
            position: Position {
                x: 1151.0,
                y: 1817.0,
                sector: crate::position_interface::SectorHandle::new(77),
                level: 1,
            },
            self_action_state: crate::element::ActionState::MovingFast,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(Position {
            x: 1171.3004,
            y: 1846.5278,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        });
        tick.enemy_detectable_positions.push((342, enemy_position));

        ai.event_view_standard_procedure(
            &sim,
            342,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingReactiontimeRunning
        );
        assert_eq!(
            ai.base
                .outbox
                .actor
                .orders
                .last()
                .expect("moving-fast EventView queues GoNear")
                .tolerance,
            161.0
        );
    }

    #[test]
    fn moving_fast_event_view_distance_uses_stretched_world_3d_positions() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(70);
        let enemy_position = Position {
            x: 57.0,
            y: 245.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };
        let mut enemy_view = object_view(ObjectType::None);
        enemy_view.kind = EntityKind::Pc;
        enemy_view.is_pc = true;
        enemy_view.position = enemy_position;
        enemy_view.elevation = 36.001007;
        enemy_view.detection_position =
            crate::coordinates::MapPoint::new(enemy_position.x, enemy_position.y);
        enemy_view.detection_position_world = crate::coordinates::WorldPoint3D::new(
            enemy_position.x,
            enemy_position.y + 36.001007,
            36.001007,
        );
        let mut views = AiEntityViewMap::new();
        views.insert(132, enemy_view);
        let ctx = AiContext {
            position: Position {
                x: 277.4972,
                y: 379.12796,
                sector: crate::position_interface::SectorHandle::new(0),
                level: 0,
            },
            elevation: 1.387514,
            self_action_state: crate::element::ActionState::MovingFast,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(ctx.position);
        tick.enemy_detectable_positions.push((132, enemy_position));

        ai.event_view_standard_procedure(
            &sim,
            132,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base
                .outbox
                .actor
                .orders
                .last()
                .expect("moving-fast EventView queues GoNear")
                .tolerance,
            94.0
        );
    }

    #[test]
    fn event_view_near_gate_uses_world_y_and_elevation() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        // Make the immediate BattleDecisions path terminate predictably once
        // it observes the empty visible-enemy list.
        ai.combat_trainer = true;

        let mut enemy_view = object_view(ObjectType::None);
        enemy_view.kind = EntityKind::Pc;
        enemy_view.is_pc = true;
        enemy_view.position = Position {
            x: 609.0,
            y: 2299.0,
            sector: None,
            level: 2,
        };
        enemy_view.elevation = 150.001;
        enemy_view.detection_position = crate::coordinates::MapPoint::new(609.0, 2299.0);
        enemy_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(609.0, 2449.001, 150.001);
        let mut views = AiEntityViewMap::new();
        let mut owner_view = object_view(ObjectType::None);
        owner_view.kind = EntityKind::Soldier;
        owner_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(575.6, 2465.001, 105.001);
        views.insert(1, owner_view);
        views.insert(12, enemy_view);
        let ctx = AiContext {
            position: Position {
                x: 575.6,
                y: 2360.0,
                sector: None,
                level: 1,
            },
            elevation: 105.001,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(ctx.position);
        tick.enemy_detectable_positions.push((
            12,
            Position {
                x: 609.0,
                y: 2299.0,
                sector: None,
                level: 2,
            },
        ));

        ai.event_view_standard_procedure(
            &sim,
            12,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        // Raw map Y differs by 61 (and would take the turn branch), while
        // Original world Y differs by only 16 after adding elevation.  The
        // 45-unit elevation component keeps the 3D max norm below 50.
        assert_ne!(
            ai.base.current_substate,
            Substate::AttackingReactiontimeTurning
        );
    }

    #[test]
    fn event_view_near_gate_uses_literal_target_position_during_door_pass() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(112);

        let mut enemy_view = object_view(ObjectType::None);
        enemy_view.kind = EntityKind::Pc;
        enemy_view.is_pc = true;
        // AI Position(enemy): the destination side of the active door pass,
        // close enough to take the immediate-battle branch if used here.
        enemy_view.position = Position {
            x: 663.75,
            y: 1421.5,
            sector: None,
            level: 2,
        };
        // GetPosition(enemy): the still-interpolating body position read by
        // MaxNormDistance, more than 50 units from the observing soldier.
        enemy_view.detection_position = crate::coordinates::MapPoint::new(560.9536, 1422.7441);
        enemy_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(560.9536, 1552.7451, 130.001);
        enemy_view.elevation = 130.001;
        let mut views = AiEntityViewMap::new();
        views.insert(170, enemy_view);
        let ctx = AiContext {
            position: Position {
                x: 657.0,
                y: 1400.0,
                sector: None,
                level: 3,
            },
            elevation: 143.06665,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(Position {
            x: 654.72314,
            y: 1403.2888,
            sector: None,
            level: 3,
        });
        tick.enemy_detectable_positions.push((
            170,
            Position {
                x: 663.75,
                y: 1421.5,
                sector: None,
                level: 2,
            },
        ));

        ai.event_view_standard_procedure(
            &sim,
            170,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingReactiontimeTurning
        );
        assert!(ai.base.list_us.is_empty());
    }

    #[test]
    #[should_panic(
        expected = "officer 1 EVENT_SEES_SOLDIER requires target 42 in camp soldier roster"
    )]
    fn review_officer_sees_soldier_requires_target_in_live_soldier_roster() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.soldier_profile_rank = ProfileRank::Officer;
        ai.set_state(AiState::Default, Substate::DefaultOnPost);
        ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventSeesSoldier, 42),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    fn review_call_go_to_officer_preserves_original_boolean_gate() {
        let sim = crate::sim_rng::test_context();
        let stimulus = Stimulus::with_human(StimulusType::CallGoToOfficer, 42);

        let mut available = EnemyAi::new(1);
        available.soldier_profile_rank = ProfileRank::Soldier;
        assert!(available.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        ));
        assert_eq!(
            available.base.current_substate,
            Substate::SeekingCharlySentToOfficer
        );
        assert_eq!(available.base.antagonist, 42);
        assert!(available.reported_to_officer);

        let mut busy = EnemyAi::new(2);
        busy.soldier_profile_rank = ProfileRank::Soldier;
        busy.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        assert!(!busy.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        ));
    }

    #[test]
    fn found_charly_assigns_friend_only_after_speech_returns() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.antagonist = 90;
        ai.set_state(AiState::Seeking, Substate::SeekingGroupCalledByOfficer);
        ai.base.outbox.reentrant.owner_work.clear();

        let mut charly = object_view(ObjectType::None);
        charly.kind = EntityKind::Soldier;
        charly.rank = ProfileRank::Soldier;
        charly.ai_state = AiState::Seeking;
        charly.ai_substate = Substate::SeekingGroupCalledByOfficer;
        let mut views = AiEntityViewMap::new();
        views.insert(42, charly);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.event_sees_charly_standard_procedure(&sim, 42, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.friend_in_trouble, 0);
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [
                AiOwnerWork::StateChange(_),
                AiOwnerWork::ActorEffects(effects),
                AiOwnerWork::Speech(AiSpeechAttempt {
                    remark: Remark::FoundCharly,
                    ..
                }),
                AiOwnerWork::ResumeSendCharlyAfterSpeech { charly: 42 }
            ] if effects.unalert_near_charly_seekers
                == Some(CharlySeekerTarget::Npc(42))
        ));
    }

    #[test]
    fn sync_reunion_uses_enroute_partners_last_waypoint() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(89);
        ai.set_state(AiState::Default, Substate::DefaultLookingSidewardsForCharly);
        ai.base.synchronize_charly = 96;
        ai.base.synchronize_index = 3;
        ai.base.macro_in_progress = true;
        ai.base.macro_command = vec![MacroOpcode::Wait as u8, 100, 0];
        ai.base.macro_command_offset = 0;
        ai.base.number_of_remaining_macro_bytes = 3;

        // RHArtificialMalignity::InitializeFriendCheck checks *last* while an
        // actor is still SUBSTATE_DEFAULT_ENROUTE.  Being stationary at the
        // requested current waypoint is not enough: the actor has not yet
        // crossed the reach-point boundary that updates the observable path
        // state.
        let mut partner = object_view(ObjectType::None);
        partner.kind = EntityKind::Soldier;
        partner.ai_state = AiState::Default;
        partner.ai_substate = Substate::DefaultEnroute;
        partner.macro_in_progress = false;
        partner.path_current_waypoint_index = 3;
        partner.path_last_waypoint_index = 2;
        partner.is_moving_map = false;
        let mut views = AiEntityViewMap::new();
        views.insert(96, partner);
        let ctx = AiContext {
            frame: 1_072,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.event_sees_charly_standard_procedure(&sim, 96, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_substate, Substate::DefaultSynchronizing);
        assert_eq!(ai.base.macro_command_offset, 0);
        assert_eq!(ai.base.number_of_remaining_macro_bytes, 3);
        assert!(!ai.base.macro_timer_is_running);
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::RegisterSynchronizingActor {
                target: 96,
                actor: 89,
            }]
        ));
    }

    #[test]
    fn got_hit_uses_live_swordfight_relationship_not_stale_ai_substate() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);

        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Pc;
        attacker.position = Position::default();
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            is_swordfighting: false,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 1,
                ..crate::ai_enemy::FighterSnapshot::default()
            });

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert!(
            !ai.base.outbox.actor.has_boundary_work(),
            "all actor calls authored before EVENT_GOTHIT's final SetViewStatus must be closed as a synchronous prefix"
        );
        assert!(matches!(
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .rev()
                .nth(1),
            Some(crate::ai::AiOwnerWork::ActorEffects(effects)) if effects.halt
        ));
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.last(),
            Some(crate::ai::AiOwnerWork::SetEyeStatus(
                crate::element::EyeStatus::DieOrGetUnconscious
            ))
        ));
    }

    #[test]
    fn got_hit_while_swordfighting_requests_direct_entry_against_new_attacker() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);

        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        attacker.camp = Camp::Royalists;
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 1,
                opponent_handles: vec![3],
                ..crate::ai_enemy::FighterSnapshot::default()
            });

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.outbox.actor.enter_swordfight,
            Some(EnterSwordfightRequest::Direct(2)),
            "Original calls EnterSwordFight directly from EVENT_GOTHIT"
        );
    }

    #[test]
    fn got_hit_while_menacing_sets_hit_animation_direction_goal_without_turn_order() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Menacing, Substate::MenacingPcInComa);

        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        attacker.position = Position {
            x: 716.74176,
            y: 252.32974,
            sector: None,
            level: 0,
        };
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            position: Position {
                x: 756.42523,
                y: 205.49872,
                sector: None,
                level: 0,
            },
            direction: 15,
            is_swordfighting: false,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.outbox.actor.set_direction,
            Some(9),
            "Original SetDirection faces the hitter while RECEIVE_HIT_DAMAGE remains installed"
        );
        assert!(
            ai.base.outbox.actor.orders.is_empty(),
            "direct SetDirection must not launch a standalone Turn sequence"
        );
        assert_eq!(
            ai.base.outbox.actor.enter_swordfight,
            Some(EnterSwordfightRequest::RaiseSword)
        );
    }

    #[test]
    fn got_hit_while_swordfighting_ignores_existing_opponent() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        attacker.camp = Camp::Royalists;
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 1,
                opponent_handles: vec![2],
                ..crate::ai_enemy::FighterSnapshot::default()
            });

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
    }

    #[test]
    #[should_panic(expected = "soldier 1 EVENT_GOTHIT requires attacker 2 entity view")]
    fn got_hit_while_swordfighting_requires_attacker_entity_view() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext {
            is_swordfighting: true,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 1,
                ..crate::ai_enemy::FighterSnapshot::default()
            });

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );
    }

    #[test]
    fn got_hit_by_friend_does_not_require_fighter_snapshot() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        attacker.camp = Camp::Lacklandists;
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
    }

    #[test]
    #[should_panic(expected = "soldier 1 EVENT_GOTHIT requires self fighter snapshot")]
    fn got_hit_while_swordfighting_requires_self_fighter_snapshot() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        attacker.camp = Camp::Royalists;
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    fn got_hit_can_begin_close_swordfight_from_default_state() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        assert_eq!(ai.base.current_state, AiState::Default);

        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Pc;
        attacker.is_pc = true;
        attacker.camp = Camp::Royalists;
        attacker.position = Position::default();
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 1,
                ..crate::ai_enemy::FighterSnapshot::default()
            });

        ai.think_alerting_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventGotHit, 2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        let engage = ai.base.outbox.actor.enter_swordfight.or_else(|| {
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .and_then(|effects| effects.enter_swordfight),
                    _ => None,
                })
        });
        assert_eq!(engage, Some(EnterSwordfightRequest::Engage(2)));
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    }

    #[test]
    fn seeing_shadow_raises_music_alert_without_accelerating_view_refresh() {
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext {
            posture: Posture::Upright,
            ..AiContext::default()
        };

        ai.event_sees_shadow_standard_procedure(
            &Position {
                x: 10.0,
                y: 20.0,
                sector: None,
                level: 0,
            },
            &ctx,
            &AiPerTickData::stub(),
        );

        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
        assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
        assert_eq!(ai.base.current_substate, Substate::DefaultLookingShadow);
    }

    fn ctx_with_object(object_type: ObjectType) -> AiContext {
        let mut views = AiEntityViewMap::new();
        views.insert(2, object_view(object_type));
        AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            posture: Posture::Upright,
            ..AiContext::default()
        }
    }

    #[test]
    fn event_sees_runtime_money_objects_reacts_but_bonus_purse_is_ignored() {
        for object_type in [ObjectType::Purse, ObjectType::Coin] {
            let mut ai = EnemyAi::new(1);
            let ctx = ctx_with_object(object_type);

            ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

            assert_eq!(ai.base.current_state, AiState::Wondering);
            assert_eq!(
                ai.base.current_substate,
                Substate::WonderingMoneyReactiontime
            );
            assert_eq!(ai.base.interesting_object, 2);
        }

        let mut ai = EnemyAi::new(1);
        let ctx = ctx_with_object(ObjectType::BonusPurse);

        ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
        assert_eq!(ai.base.interesting_object, 0);
    }

    #[test]
    fn event_sees_runtime_ale_reacts_but_bonus_ale_is_ignored() {
        let mut ai = EnemyAi::new(1);
        let ale_position = Position {
            x: 632.4453,
            y: 1835.14,
            sector: None,
            level: 0,
        };
        let mut ale_view = object_view(ObjectType::Ale);
        ale_view.position = ale_position;
        let mut views = AiEntityViewMap::new();
        views.insert(2, ale_view);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            posture: Posture::Upright,
            ..AiContext::default()
        };

        ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(ai.base.current_substate, Substate::WonderingAleReactiontime);
        assert_eq!(ai.base.interesting_object, 2);
        assert_eq!(ai.base.seek_position, ale_position);

        let mut ai = EnemyAi::new(1);
        let ctx = ctx_with_object(ObjectType::BonusAle);

        ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
        assert_eq!(ai.base.interesting_object, 0);
    }

    #[test]
    fn event_sees_civilian_beggar_preserves_the_legacy_slots_entity_kind() {
        let sim_context = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSeekpoint;

        let mut beggar_view = object_view(ObjectType::None);
        beggar_view.kind = EntityKind::Civilian;
        beggar_view.is_beggar = true;
        let mut views = AiEntityViewMap::new();
        views.insert(17, beggar_view);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_unexpected_event(
            &sim_context,
            &Stimulus::with_human(StimulusType::EventSeesBeggar, 17),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.outbox.actor.delete_beggar_for_all_npc,
            vec![crate::element::EntityId::Civilian(
                crate::entity_id::CivilianId(17)
            )]
        );
    }

    #[test]
    fn event_sees_current_beggar_does_not_requeue_but_still_requests_global_scrub() {
        let sim_context = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSeekpointApproachingBeggar;
        ai.beggar_to_examine = 17;

        let mut beggar_view = object_view(ObjectType::None);
        beggar_view.kind = EntityKind::Civilian;
        beggar_view.is_beggar = true;
        let mut views = AiEntityViewMap::new();
        views.insert(17, beggar_view);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_unexpected_event(
            &sim_context,
            &Stimulus::with_human(StimulusType::EventSeesBeggar, 17),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(ai.beggars_to_control.is_empty());
        assert!(ai.positions_of_beggars_to_control.is_empty());
        assert_eq!(
            ai.base.outbox.actor.delete_beggar_for_all_npc,
            vec![crate::element::EntityId::Civilian(
                crate::entity_id::CivilianId(17)
            )]
        );
    }

    #[test]
    fn event_enemy_near_assigns_stimulus_target_and_begins_swordfight() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        for substate in [
            Substate::AttackingReactiontimeTurning,
            Substate::AttackingReactiontime,
            Substate::AttackingApproachToObserve,
            Substate::AttackingObserve,
        ] {
            let mut ai = EnemyAi::new(1);
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = substate;
            ai.base.primary_target = 12;
            // The original trainer gate is exclusively on the sender.
            ai.combat_trainer = true;

            let stimulus = Stimulus::with_human(StimulusType::EventEnemyNear, 77);
            ai.think_unexpected_event(
                sim,
                &stimulus,
                &mut AiGlobalState::default(),
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.base.primary_target, 77, "substate {substate:?}");
            // begin_swordfight raises Engage before its SetState suspends
            // the actor-outbox prefix into the queued state-change owner
            // work; read the request from either place.
            let engage = ai.base.outbox.actor.enter_swordfight.or_else(|| {
                ai.base
                    .outbox
                    .reentrant
                    .owner_work
                    .iter()
                    .find_map(|work| match work {
                        crate::ai::AiOwnerWork::StateChange(notification) => notification
                            .actor_effects_before_callback
                            .as_ref()
                            .and_then(|effects| effects.enter_swordfight),
                        _ => None,
                    })
            });
            assert_eq!(
                engage,
                Some(EnterSwordfightRequest::Engage(77)),
                "substate {substate:?}"
            );
            assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        }
    }

    #[test]
    fn event_enemy_near_is_ignored_outside_original_substates() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 12;

        let stimulus = Stimulus::with_human(StimulusType::EventEnemyNear, 77);
        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.primary_target, 12);
        assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
    }

    #[test]
    fn officer_call_alert_halts_actor_without_breaking_running_macro() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(44);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultInMacro;
        ai.base.macro_in_progress = true;
        ai.base.macro_timer_is_running = true;
        ai.base.when_does_macro_timer_ring = 10_054;
        ai.current_task_priority = task_priority::ALERT;
        ai.new_task_priority = task_priority::ALERT;

        let mut officer = object_view(ObjectType::None);
        officer.kind = EntityKind::Soldier;
        officer.rank = ProfileRank::Officer;
        officer.position = Position {
            x: 100.0,
            y: 20.0,
            sector: None,
            level: 0,
        };
        let mut views = AiEntityViewMap::new();
        views.insert(91, officer);
        let ctx = AiContext {
            frame: 9_768,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let accepted = ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::CallAlert, 91),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(accepted);
        let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::StateChange(notification)
                    if notification
                        .actor_effects_before_callback
                        .as_ref()
                        .is_some_and(|effects| effects.halt)
            )
        });
        assert!(halted_before_state_change);
        assert!(ai.base.macro_in_progress);
        assert!(ai.base.macro_timer_is_running);
        assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingGroupCalledByOfficer
        );
    }

    #[test]
    fn civilian_call_alert_halts_actor_without_breaking_running_macro() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(44);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultInMacro;
        ai.base.macro_in_progress = true;
        ai.base.macro_timer_is_running = true;
        ai.base.when_does_macro_timer_ring = 10_054;

        let mut civilian = object_view(ObjectType::None);
        civilian.kind = EntityKind::Civilian;
        civilian.position = Position {
            x: 100.0,
            y: 20.0,
            sector: None,
            level: 0,
        };
        let mut views = AiEntityViewMap::new();
        views.insert(91, civilian);
        let ctx = AiContext {
            frame: 9_768,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let accepted = ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::CallAlert, 91),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(accepted);
        let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::StateChange(notification)
                    if notification
                        .actor_effects_before_callback
                        .as_ref()
                        .is_some_and(|effects| effects.halt)
            )
        });
        assert!(halted_before_state_change);
        assert!(ai.base.macro_in_progress);
        assert!(ai.base.macro_timer_is_running);
        assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
        assert_eq!(ai.base.antagonist, 91);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingWaitForAlertingCivilian
        );
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 9_788);
    }

    #[test]
    fn rejected_civilian_call_alert_still_replaces_antagonist() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(44);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingRunningToOfficer;
        ai.base.antagonist = 78;

        let mut civilian = object_view(ObjectType::None);
        civilian.kind = EntityKind::Civilian;
        let mut views = AiEntityViewMap::new();
        views.insert(91, civilian);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let accepted = ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::CallAlert, 91),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(!accepted);
        assert_eq!(ai.base.antagonist, 91);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, Substate::SeekingRunningToOfficer);
    }

    #[test]
    fn soldier_call_alert_halts_officer_without_breaking_running_macro() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(44);
        ai.soldier_profile_rank = ProfileRank::Officer;
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultInMacro;
        ai.base.macro_in_progress = true;
        ai.base.macro_timer_is_running = true;
        ai.base.when_does_macro_timer_ring = 10_054;

        let mut soldier = object_view(ObjectType::None);
        soldier.kind = EntityKind::Soldier;
        soldier.rank = ProfileRank::Soldier;
        soldier.position = Position {
            x: 100.0,
            y: 20.0,
            sector: None,
            level: 0,
        };
        let mut views = AiEntityViewMap::new();
        views.insert(91, soldier);
        let ctx = AiContext {
            frame: 9_768,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let accepted = ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::CallAlert, 91),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(accepted);
        let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::StateChange(notification)
                    if notification
                        .actor_effects_before_callback
                        .as_ref()
                        .is_some_and(|effects| effects.halt)
            )
        });
        assert!(halted_before_state_change);
        assert!(ai.base.macro_in_progress);
        assert!(ai.base.macro_timer_is_running);
        assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingOfficerWaitForAlertingSoldier
        );
    }

    #[test]
    fn couldnt_reach_running_enemy_enters_battle_overview() {
        let sim_context = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.list_us = vec![1, 2];

        ai.think_unexpected_event(
            &sim_context,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
        assert_eq!(
            ai.base.list_us,
            vec![1, 2],
            "GetBattleOverview must not rebuild the persistent friend list"
        );
    }

    #[test]
    fn same_frame_observe_move_failure_resumes_inline_roof_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingApproachToObserve;
        ai.base.primary_target = 183;
        ai.base.ai_log.push(LogLine {
            line_type: LogLineType::BattleDecision,
            info: Decision::Observe as u16,
            frame: 8_103,
        });

        let target_position = Position {
            x: 2_788.0,
            y: 1_029.0,
            sector: crate::position_interface::SectorHandle::new(53),
            level: 2,
        };
        let wait_position = Position {
            x: 2_793.0,
            y: 571.0,
            sector: crate::position_interface::SectorHandle::new(53),
            level: 2,
        };
        let mut target = object_view(ObjectType::None);
        target.kind = EntityKind::Pc;
        target.is_pc = true;
        target.position = target_position;
        let mut views = AiEntityViewMap::new();
        views.insert(183, target);
        let ctx = AiContext {
            frame: 8_103,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.avenger_on_roof_wait_positions
            .push((183, wait_position));

        let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
        ai.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunToAvengerOnRoof
        );
        assert_eq!(ai.base.seek_position, target_position);
        assert_eq!(ai.base.last_goto_destination, wait_position);
        assert!(ai.base.outbox.actor.look_sidewards.is_none());
    }

    #[test]
    fn same_frame_fight_lift_failure_preserves_inline_roof_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToLadder;
        ai.base.primary_target = 183;
        ai.base.last_synced_focus_target = Some(183);
        ai.base.timer_is_running = true;
        ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
        ai.base.when_does_timer_ring = 8_133;

        let target_position = Position {
            x: 2_788.0,
            y: 1_029.0,
            sector: crate::position_interface::SectorHandle::new(63),
            level: 3,
        };
        let wait_position = Position {
            x: 2_793.0,
            y: 571.0,
            sector: crate::position_interface::SectorHandle::new(53),
            level: 2,
        };
        let mut target = object_view(ObjectType::None);
        target.kind = EntityKind::Pc;
        target.is_pc = true;
        target.position = target_position;
        let mut views = AiEntityViewMap::new();
        views.insert(183, target);
        let ctx = AiContext {
            frame: 8_103,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.avenger_on_roof_wait_positions
            .push((183, wait_position));

        let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
        ai.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunToAvengerOnRoof
        );
        assert_eq!(ai.base.seek_position, target_position);
        assert_eq!(ai.base.last_goto_destination, wait_position);
        assert_eq!(ai.base.last_synced_focus_target, Some(183));
        assert!(!ai.base.couldnt_reachpoint);
        assert!(ai.base.outbox.actor.orders.is_empty());
        let Some(crate::ai::AiOwnerWork::ActorEffects(roof_effects)) =
            ai.base.outbox.reentrant.owner_work.last()
        else {
            panic!("roof fallback must settle at a synchronous owner boundary")
        };
        assert_eq!(roof_effects.orders.len(), 1);
        assert_eq!(roof_effects.orders[0].target_x, wait_position.x);
        assert_eq!(roof_effects.orders[0].target_y, wait_position.y);
        assert!(roof_effects.look_sidewards.is_none());
        assert!(!roof_effects.unfocus);
    }

    #[test]
    fn same_frame_roof_fallback_failure_uses_generic_overview() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunToAvengerOnRoof;
        ai.base.primary_target = 183;
        ai.base.list_us = vec![64, 79];

        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &AiContext {
                frame: 7_938,
                ..AiContext::default()
            },
            &AiPerTickData::stub(),
            None,
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
    }

    #[test]
    fn same_frame_ladder_condolation_uses_generic_overview() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToLadder;
        ai.base.primary_target = 183;
        ai.base.list_us = vec![64, 79];
        ai.base.timer_is_running = true;
        ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
        ai.base.when_does_timer_ring = 7_968;

        let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        stimulus.self_origin = crate::ai::SelfStimulusOrigin::Condolation;
        ai.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext {
                frame: 7_938,
                ..AiContext::default()
            },
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
    }

    #[test]
    fn later_ladder_failure_still_uses_generic_emergency_routine() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToLadder;
        ai.base.primary_target = 183;
        ai.base.list_us = vec![64, 79];
        ai.base.ai_log.push(LogLine {
            line_type: LogLineType::BattleDecision,
            info: Decision::Fight as u16,
            frame: 8_102,
        });

        let ctx = AiContext {
            frame: 8_103,
            ..AiContext::default()
        };
        let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
        ai.think_unexpected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
    }

    #[test]
    fn same_frame_fight_lift_failure_without_roof_wait_resumes_observe_then_overview() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToLadder;
        ai.base.primary_target = 183;
        ai.base.last_synced_focus_target = Some(183);
        ai.base.timer_is_running = true;
        ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
        ai.base.when_does_timer_ring = 7_968;

        let target_position = Position {
            x: 2_762.2429,
            y: 882.6701,
            sector: crate::position_interface::SectorHandle::new(53),
            level: 2,
        };
        let mut target = object_view(ObjectType::None);
        target.kind = EntityKind::Pc;
        target.is_pc = true;
        target.position = target_position;
        let mut views = AiEntityViewMap::new();
        views.insert(183, target);
        let ctx = AiContext {
            frame: 7_938,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let tick = AiPerTickData::stub();

        let mut engine_completion = Stimulus::new(StimulusType::EventCouldntReachPoint);
        engine_completion.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
        ai.think_unexpected_event(
            &sim,
            &engine_completion,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingApproachToObserve
        );
        assert_eq!(ai.base.when_does_timer_ring, 7_988);
        assert!(ai.base.timer_is_running);
        assert!(ai.base.couldnt_reachpoint);
        assert_eq!(ai.base.last_synced_focus_target, Some(183));

        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
    }

    #[test]
    fn later_roof_failure_still_uses_generic_emergency_routine() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(64);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunToAvengerOnRoof;
        ai.base.primary_target = 183;
        ai.base.list_us = vec![64, 79];
        let ctx = AiContext {
            frame: 8_104,
            ..AiContext::default()
        };
        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            ai.base.outbox.actor.look_sidewards,
            Some(LookDirection::Left)
        );
    }

    #[test]
    fn couldnt_reach_seeking_body_examines_queued_body_before_starting_seek_area() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(206);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.other_bodies_to_examine.push(207);

        let alternate_position = Position {
            x: 658.0,
            y: 2910.0,
            sector: crate::position_interface::SectorHandle::new(18),
            level: 0,
        };
        let mut alternate_body = object_view(ObjectType::None);
        alternate_body.kind = EntityKind::Soldier;
        alternate_body.position = alternate_position;
        // `ExamineOtherBodies` prunes the queue with `IsOutOfOrder()`, not
        // with `IsAbleToFight()` — a KO'd body is what keeps this entry
        // queued (`RHelementactorhuman.cpp:13271`).
        alternate_body.is_unconscious = true;
        alternate_body.is_able_to_fight = false;
        let mut views = AiEntityViewMap::new();
        views.insert(207, alternate_body);
        let ctx = AiContext {
            position: Position {
                x: 792.0,
                y: 2612.0,
                sector: crate::position_interface::SectorHandle::new(44),
                level: 0,
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.detected_body, 207);
        assert_eq!(ai.base.seek_position, alternate_position);
        assert_eq!(ai.base.current_substate, Substate::SeekingBody);
        assert!(ai.my_seek_points.is_empty());
    }

    /// `RHArtificialMalignity::ExamineOtherBodies`
    /// (`RHartificialmalignity.cpp:20128`) prunes the queue head while
    /// `!IsOutOfOrder()`. `IsOutOfOrder` (`RHelementactorhuman.cpp:13271`) is
    /// a body-state predicate, *not* the combat-readiness `IsAbleToFight`.
    /// Civilians never report able-to-fight, so proxying the two keeps a woken
    /// civilian sleeper queued forever: the soldier re-examines the body it is
    /// already standing next to, `GoNear` short-circuits to
    /// `EVENT_REACHPOINT`, and the seek collapses into `ReturnToDuty`.
    #[test]
    fn examine_other_bodies_prunes_recovered_civilian_that_cannot_fight() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(206);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        // Head of the queue: a civilian that woke up. `is_able_to_fight` is
        // false for every civilian, but `IsOutOfOrder()` is false now.
        ai.other_bodies_to_examine.push(207);
        // Behind it: a soldier that is genuinely still down.
        ai.other_bodies_to_examine.push(208);

        let mut recovered = object_view(ObjectType::None);
        recovered.kind = EntityKind::Civilian;
        recovered.is_able_to_fight = false;

        let down_position = Position {
            x: 658.0,
            y: 2910.0,
            sector: crate::position_interface::SectorHandle::new(18),
            level: 0,
        };
        let mut still_down = object_view(ObjectType::None);
        still_down.kind = EntityKind::Soldier;
        still_down.position = down_position;
        still_down.is_able_to_fight = false;
        still_down.is_unconscious = true;

        let mut views = AiEntityViewMap::new();
        views.insert(207, recovered);
        views.insert(208, still_down);
        let ctx = AiContext {
            position: Position {
                x: 792.0,
                y: 2612.0,
                sector: crate::position_interface::SectorHandle::new(44),
                level: 0,
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.examine_other_bodies(&ctx, &AiPerTickData::stub()));
        assert_eq!(
            ai.base.detected_body, 208,
            "the recovered civilian must be pruned, not examined"
        );
        assert_eq!(ai.base.seek_position, down_position);
        assert!(ai.other_bodies_to_examine.is_empty());
        let _ = &sim;
    }

    /// The predicate itself, in both directions: `IsOutOfOrder()` is the OR of
    /// the five body states (plus PC coma) and is independent of
    /// `IsAbleToFight()`.
    #[test]
    fn is_out_of_order_is_not_the_complement_of_is_able_to_fight() {
        // Civilian that is up and about: never able to fight, but in order.
        let mut civilian = object_view(ObjectType::None);
        civilian.kind = EntityKind::Civilian;
        civilian.is_able_to_fight = false;
        assert!(!civilian.is_out_of_order());

        // Netted / tied / carried / KO'd / dead all count as out of order even
        // when the combat-readiness flag says otherwise.
        for apply in [
            (|v: &mut AiEntityView| v.stuck_under_net = true) as fn(&mut AiEntityView),
            |v: &mut AiEntityView| v.posture = Posture::Tied,
            |v: &mut AiEntityView| v.posture = Posture::Carried,
            |v: &mut AiEntityView| v.is_unconscious = true,
            |v: &mut AiEntityView| v.is_dead = true,
        ] {
            let mut soldier = object_view(ObjectType::None);
            soldier.kind = EntityKind::Soldier;
            soldier.is_able_to_fight = true;
            apply(&mut soldier);
            assert!(soldier.is_out_of_order());
        }

        // The coma arm is PC-only.
        let mut comatose = object_view(ObjectType::None);
        comatose.kind = EntityKind::Pc;
        comatose.in_coma = true;
        assert!(comatose.is_out_of_order());
        let mut soldier_flagged_coma = object_view(ObjectType::None);
        soldier_flagged_coma.kind = EntityKind::Soldier;
        soldier_flagged_coma.in_coma = true;
        assert!(!soldier_flagged_coma.is_out_of_order());
    }

    #[test]
    fn couldnt_reach_seeking_body_centers_fallback_on_actor_not_stale_body() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(206);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.base.seek_position = Position {
            x: 2_000.0,
            y: 2_000.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        };
        let actor_position = Position {
            x: 792.0,
            y: 2612.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        };
        let actor_seek_point = Position {
            x: 652.0,
            y: 2_928.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        };
        let stale_body_seek_point = Position {
            x: 2_020.0,
            y: 2_000.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        };
        let point = |id, position| SeekPoint {
            position,
            frame_when_full_interest: 0,
            directions: vec![0],
            last_calculated_interest: 100,
            locked: false,
            id,
        };
        let mut global = AiGlobalState::default();
        global.seek_points = vec![point(0, actor_seek_point), point(1, stale_body_seek_point)];
        let ctx = AiContext {
            position: actor_position,
            self_is_soldier: true,
            ..AiContext::default()
        };

        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventCouldntReachPoint),
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.seek_center, actor_position);
        assert_eq!(ai.actual_seek_point, Some(0));
        assert_eq!(ai.base.last_goto_destination, actor_seek_point);
        assert_ne!(ai.base.last_goto_destination, stale_body_seek_point);
    }

    #[test]
    fn event_hear_faces_noise_position_projection_not_recorded_actor_elevation() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(70);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSeekpoint;

        // Trace-shaped boundary from SuN/Profile_004/Savegame_016 r013
        // frame 895. Sector 0 has no projection area, so PositionToPoint3D
        // leaves the noise at z=0 even though the producing PC recorded its
        // own elevation (36) on RHnoise.
        let noise = Noise {
            origin: Position {
                x: f32::from_bits(0x428b_1027),
                y: f32::from_bits(0x43af_c940),
                sector: crate::position_interface::SectorHandle::new(0),
                level: 0,
            },
            noise_type: NoiseType::ZingZing,
            volume: 200,
            elevation: 36,
            element_id: 133,
        };
        let ctx = AiContext {
            position: Position {
                x: f32::from_bits(0x4326_9901),
                y: f32::from_bits(0x438f_54f0),
                sector: crate::position_interface::SectorHandle::new(0),
                level: 0,
            },
            self_body_position_world: crate::coordinates::WorldPoint3D::new(
                f32::from_bits(0x4326_9901),
                f32::from_bits(0x43a1_5511),
                f32::from_bits(0x4210_0107),
            ),
            elevation: f32::from_bits(0x4210_0107),
            direction: 4,
            ..AiContext::default()
        };

        let scalar_dx = noise.origin.x - ctx.position.x;
        let scalar_dy =
            (noise.origin.y - ctx.position.y) + (noise.elevation as f32 - ctx.elevation);
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_with_aspect(
                scalar_dx,
                scalar_dy,
                crate::position_interface::ASPECT_RATIO,
            ),
            10,
            "the replaced scalar-elevation shortcut must select the adjacent sector"
        );

        ai.event_hear_standard_procedure(&sim, &noise, &ctx, &AiPerTickData::stub());

        let turn = ai
            .base
            .outbox
            .actor
            .orders
            .iter()
            .find(|intent| intent.order_type == OrderType::Turning)
            .expect("the seeking EventHear arm must author a Turn");
        assert_eq!(turn.explicit_direction, Some(11));
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingHeardstepsReactiontime
        );
    }

    /// Original `RHartificialmalignity.cpp:5442-5452` sweeps the waiting
    /// soldier's own position when the avenger it is watching for goes out of
    /// view and no fighters remain. Rust used the remembered avenger position
    /// (`mposSeekPosition`), which shifts the seek center and therefore the
    /// near-point membership that drives the phase-4 selection draw count.
    #[test]
    fn avenger_roof_out_of_view_seeks_from_live_owner_position() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(178);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingWaitForAvengerOnRoof;
        ai.base.primary_target = 0;
        // Keep the stale avenger center far from the live position so a
        // regression cannot accidentally pick the same seek points.
        ai.base.seek_position = Position {
            x: 240.0,
            y: 860.0,
            ..Position::default()
        };

        let live_position = Position {
            x: 1_800.0,
            y: 2_200.0,
            ..Position::default()
        };
        let mut global = AiGlobalState::default();
        global.seek_points = [(1_810.0, 2_200.0), (1_820.0, 2_200.0)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| crate::ai::SeekPoint {
                position: Position {
                    x,
                    y,
                    ..Position::default()
                },
                frame_when_full_interest: 0,
                directions: vec![0],
                last_calculated_interest: 100,
                locked: false,
                id: id as u16,
            })
            .collect();

        let ctx = AiContext {
            frame: 12_345,
            position: live_position,
            ..AiContext::default()
        };

        ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventOutOfView, 42),
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.seek_center, live_position);
        assert_ne!(ai.seek_center, ai.base.seek_position);
    }
}
