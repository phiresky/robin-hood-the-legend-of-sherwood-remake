//! `EnemyAi` event handlers: `think_unexpected_event`,
//! `think_alerting_event`, the per-event standard procedures, and
//! the post-event helpers (`get_angry_about_apple`,
//! `couldnt_reachpoint_emergency_routine`,
//! `event_sees_charly_standard_procedure`).
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the file manageable.

use crate::ai::*;
use crate::parameters_ai;

use super::util::enemy_is_below_me;
use super::{EnemyAi, ProfileRank, SeekFlags, UNDEFINED_DIRECTION, combat, task_priority};

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
                    self.base.outbox.actor.unalert_near_charly_seekers =
                        Some(CharlySeekerTarget::Npc(charly));

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
                                self.base.outbox.actor.unalert_near_charly_seekers =
                                    Some(CharlySeekerTarget::Npc(charly));
                                self.base
                                    .say_with_flags(Remark::FoundCharly, SpeechFlags::MYTALK_1);
                                self.base.friend_in_trouble = charly;
                                self.base.face_entity(charly, ctx);
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
                self.base.outbox.actor.unalert_near_charly_seekers =
                    Some(CharlySeekerTarget::Npc(charly));
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

    pub(super) fn think_unexpected_event(
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
                    self.search_charly(sim, ctx, tick);
                }
                return true;
            }
            StimulusType::EventOutOfView => {
                if self.base.current_state == AiState::Attacking
                    && let StimulusInfo::Human(enemy) = stimulus.info
                {
                    // Lost sight of enemy while attacking.
                    match self.base.current_substate {
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

                        // Wait-for-avenger substates.
                        Substate::AttackingWaitForAvengerOnRoof => {
                            self.reinitialize_them_list(ctx, tick);
                            if self.list_them.is_empty() {
                                self.seek_area(
                                    sim,
                                    self.base.seek_position,
                                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                                    SeekFlags::empty(),
                                    UNDEFINED_DIRECTION,
                                    global,
                                    ctx,
                                    tick,
                                );
                            } else {
                                self.get_battle_overview(0x0001, ctx, tick);
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
                        self.seek_area(
                            sim,
                            self.base.seek_position,
                            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                            SeekFlags::empty(),
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
                    }
                    Substate::AttackingRunningToEnemy
                    | Substate::AttackingWalkingToEnemy
                    | Substate::AttackingChargingEnemy => {
                        // Can't reach enemy — try observe instead
                        self.reinitialize_them_list(ctx, tick);
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    }
                    Substate::AttackingObserve => {
                        // Ignore.
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
                        self.left_combat_neighbour = 0;
                        self.right_combat_neighbour = 0;
                        self.base.launch_timer(3, ctx.frame);
                    }
                }
            }

            StimulusType::EventSwordStrike => {
                // ConsiderToBeginParade is handled in the engine melee layer
                // (EngineInner::consider_to_begin_parade) rather than here, because
                // it needs direct access to entity state, weapon profiles, and
                // the sequence manager.  It's dispatched from warn_for_strike
                // when a sword strike starts targeting this soldier.
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
                        if caller.is_civilian() {
                            if self.base.current_state != AiState::Default {
                                return false;
                            }
                            self.base.antagonist = civilian;
                            self.base.stop_all();
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

                        self.base.antagonist = civilian;
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
                                self.base.stop_all();
                                self.current_task_priority = self.new_task_priority;
                                self.gather_position_instructed = false;
                                self.base.friends_are_alerted = true;
                                self.officers_position = caller.position;
                                self.base.face_position(caller.position);
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
                                self.base.stop_all();
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
                if self.base.current_substate == Substate::AttackingSwordfightSpecialStrike {
                    let remark = if self.is_vip {
                        Remark::VipGoodStrikeCombat
                    } else {
                        Remark::GoodStrikeCombat
                    };
                    self.base.say(remark);
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
                    // DeleteDetectableForAllNPC(beggar, DETECTABLE_BEGGAR)
                    // so every other seek-area soldier's BEGGAR list loses
                    // this PC, guaranteeing a single soldier handles the
                    // identification. Queue it through the engine drain
                    // (`engine/ai.rs` process pending orders) since we
                    // can't touch other entities from here.
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
                            sector: SectorHandle::new(wp.sector),
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
                self.base
                    .coordinate_patrol(&stimulus.info, ctx, tick.patrol_chief_position);
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
                                        ctx.entity_view(enemy).map(|v| (v.position, v.elevation)),
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

                        // SetStates(UPRIGHT, HOLDING_SHIELD) + UpdateShield
                        // are redundant with the sequence dispatch, which
                        // `dispatch_raise_shield_instantly` processes at
                        // post-think; shield obstacles are recomputed
                        // every frame by
                        // `EngineInner::update_shield_obstacles`.

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
                    debug_assert!(!target_is_friend);
                    if target_is_friend {
                        tracing::warn!(
                            me = self.base.me,
                            enemy,
                            "EVENT_ENTER_SWORDFIGHT target is friendly; matching reference release behavior and entering anyway"
                        );
                    }
                    let allowed_to_attack = self.is_allowed_to_attack(enemy, ctx, tick);
                    debug_assert!(allowed_to_attack);
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
                // The sender (`hey_folks_look_there`) filters by the
                // target's ai_state from a tick-start snapshot. That
                // snapshot is stale by the time this stimulus is
                // drained from `pending_cross_npc_actions`: an NPC
                // that transitioned into `Attacking` earlier in the
                // same tick still receives the queued look-there.
                // Without a receiver-side gate, the handler's
                // unconditional `set_state(Wondering, WonderingWatching)`
                // kicks the NPC back out of combat, causing the
                // flip-flop loop ("spotted PC → Attacking → yanked to
                // Wondering → spotted PC → Attacking → …") that the
                // log traces showed every ~50 ms while running to engage.
                // Mirror the sender-side filter here — the live
                // `GetAIState()` check in the reference is equivalent
                // because its stimulus dispatch is synchronous.
                let state_ok = matches!(
                    self.base.current_state,
                    AiState::Default | AiState::Wondering
                ) || matches!(
                    self.base.current_substate,
                    Substate::SeekingJustWatching | Substate::SeekingJustWatchingSidewards
                );
                if state_ok
                    && let StimulusInfo::Hint(ref hint) = stimulus.info
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
                        let attacker_is_friend = ctx
                            .entity_view(attacker)
                            .map(|v| v.camp == ctx.camp)
                            .unwrap_or(false);
                        let already_opponent = self
                            .find_fighter(self.base.me, tick)
                            .map(|f| f.has_as_opponent(attacker))
                            .unwrap_or(false);
                        if !attacker_is_friend && !already_opponent {
                            self.base.outbox.actor.enter_swordfight =
                                Some(EnterSwordfightRequest::Engage(attacker));
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
                        self.base.face_entity(attacker, ctx);
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
                        self.base.outbox.recovery.set_eye_status =
                            Some(crate::element::EyeStatus::DieOrGetUnconscious);
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
        self.enemy_seen_below =
            enemy_is_below_me(ctx, enemy_view.map(|v| (v.position, v.elevation)));

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
        let enemy_pos = enemy_view
            .map(|v| v.position)
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

        // HeyFolksLookThere(enemy_pos, VIEW_LOOK_THERE_RADIUS).
        self.hey_folks_look_there(&enemy_pos, 100, ctx);

        // Already sprinting? Stay in MovingFast, just commit the target
        // and re-issue the run-to. Skips the StopAll/Say path entirely so
        // the sprint animation chains straight into the engage.
        if ctx.self_action_state == crate::element::ActionState::MovingFast {
            self.set_state(AiState::Attacking, Substate::AttackingReactiontimeRunning);
            self.base.primary_target = enemy;
            self.base.outbox.actor.set_focus(enemy);
            self.reinitialize_them_list(ctx, tick);
            // GoNear(Position(pEnemy), Distance/3, GOTO_RUN)
            let dx = enemy_pos.x - ctx.position.x;
            let dy =
                (enemy_pos.y - ctx.position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let distance = (dx * dx + dy * dy).sqrt();
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
        let max_norm_dist = {
            // `MaxNormDistance` applies `StretchY(INVERSE_ASPECT_RATIO)`
            // before `MaxNorm()`.
            let dx = (enemy_pos.x - ctx.position.x).abs();
            let dy = (enemy_pos.y - ctx.position.y).abs()
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            dx.max(dy)
        };

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
                        self.base.face_position_at_elevation_with_ctx(
                            noise.origin,
                            noise.elevation as f32,
                            ctx,
                        );
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
                    self.base.face_position_at_elevation_with_ctx(
                        noise.origin,
                        noise.elevation as f32,
                        ctx,
                    );
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
                    self.base.face_position_at_elevation_with_ctx(
                        noise.origin,
                        noise.elevation as f32,
                        ctx,
                    );
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
                self.base.face_position_at_elevation_with_ctx(
                    noise.origin,
                    noise.elevation as f32,
                    ctx,
                );
                self.base.launch_timer(50, ctx.frame);
            }

            NoiseType::Logs | NoiseType::Drawbridge
                if self.base.current_state == AiState::Default =>
            {
                self.base.stop_all();
                self.set_state(AiState::Wondering, Substate::WonderingWatching);
                self.base.seek_position = noise.origin;
                self.base.face_position_at_elevation_with_ctx(
                    noise.origin,
                    noise.elevation as f32,
                    ctx,
                );
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
                    self.base.outbox.actor.unalert_near_charly_seekers =
                        Some(CharlySeekerTarget::Npc(body));
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
        self.hey_folks_look_there(&body_pos, 100, ctx);
        self.seen_dead_body = false;

        self.base.stop_all();
        // Remember the body and its position.
        self.base.seek_position = body_pos;
        self.base.detected_body = body;
        self.base.outbox.actor.set_focus(body);

        self.set_state(AiState::Seeking, Substate::SeekingBodyReactiontime);

        // Turn to look at the body.
        self.base.face_position(body_pos);
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // Post-SetState charly check fires the unalert cascade for
        // non-mid-seek discoveries too. The body we just saw IS charly,
        // so the sweep target is the body handle.
        if b_hey_this_is_charly {
            self.base.outbox.actor.unalert_near_charly_seekers =
                Some(CharlySeekerTarget::Npc(body));
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
            self.base.face_position(seek);
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
            self.base.face_position(seek);
            // Focus on the interesting object — locks the eye-tracking
            // cone onto the arrow's interesting object so the detection
            // cone narrows along the threat axis.
            if self.base.interesting_object != 0 {
                self.base
                    .outbox
                    .actor
                    .set_focus(self.base.interesting_object);
            }
            self.hey_folks_look_there(pos, 200, ctx);
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
                    self.base.face_position(view.position);
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
                    self.base.face_position(view.position);
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
        self.base.face_position_with_ctx(*pos, ctx);
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
                self.alert_officer(sim, self.base.seek_position, 0, ctx, tick);
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
        self.base.face_position(*pos);
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

        self.base.face_position(*pos);
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
        // EVENT_COULDNT_REACHPOINT once the posture clears. Same pattern
        // as `return_to_duty`'s busy gate. The PassDoor / Fall
        // sequence-element check from the helper
        // is deferred (the AI doesn't see the live sequence manager
        // here); the posture cases cover the dominant path.
        if matches!(
            ctx.posture,
            Posture::Flying | Posture::OnLadder | Posture::OnWall,
        ) {
            self.base.non_script_lock(crate::ai::AiLockFlags::BUSY);
            self.base.was_busy = true;
            self.base
                .outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventCouldntReachPoint);
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
    use std::sync::Arc;

    fn object_view(object_type: ObjectType) -> AiEntityView {
        AiEntityView {
            position: Position {
                x: 10.0,
                y: 20.0,
                sector: None,
                level: 0,
            },
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
            passing_door: false,
            obstacle_idx: None,
            in_building: false,
            building_sector: None,
            script_locked: false,
            forecasted_destination: Position::default(),
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
    fn got_hit_uses_live_swordfight_relationship_not_stale_ai_substate() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);

        let mut attacker = object_view(ObjectType::None);
        attacker.kind = EntityKind::Soldier;
        let mut views = AiEntityViewMap::new();
        views.insert(2, attacker);
        let ctx = AiContext {
            is_swordfighting: false,
            entity_views: Arc::new(views),
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

        assert!(
            ai.base.outbox.actor.halt,
            "Original StopAll arm runs once the live opponent list is empty"
        );
        assert_eq!(
            ai.base.outbox.recovery.set_eye_status,
            Some(crate::element::EyeStatus::DieOrGetUnconscious)
        );
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
            entity_views: Arc::new(views),
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
        let ctx = ctx_with_object(ObjectType::Ale);

        ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(ai.base.current_substate, Substate::WonderingAleReactiontime);
        assert_eq!(ai.base.interesting_object, 2);

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
            entity_views: Arc::new(views),
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
            assert_eq!(
                ai.base.outbox.actor.enter_swordfight,
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
}
