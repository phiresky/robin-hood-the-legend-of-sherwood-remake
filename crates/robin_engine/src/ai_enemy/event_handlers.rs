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
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_GOOD_STRIKE_LIFECYCLE",
            [
                "PARITY_DEBUG_GOOD_STRIKE_FRAME",
                "PARITY_DEBUG_GOOD_STRIKE_CREATION_ORDER",
            ],
        )
    });
    gate.enabled() && gate.matches_required([Some(ctx.frame), ctx.original_creation_order])
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
        charly: AiEntityHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // the encountered soldier's metadata (rank, reported-to-officer, substate) comes
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

                    // If the encountered actor has soldier rank, acquire him and wait.
                    let charly_is_soldier = charly_view
                        .as_ref()
                        .map(|v| v.is_soldier() && v.rank == ProfileRank::Soldier)
                        .unwrap_or(false);
                    if charly_is_soldier {
                        self.base.say(Remark::FoundCharly);
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: charly.get(),
                                stimulus_type: StimulusType::CallGoToOfficer,
                                info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                        self.base.antagonist = Some(charly);
                        self.base.face_entity(charly, ctx);
                        self.set_state_with_timer(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForCharly,
                            10,
                            ctx,
                        );
                        return;
                    }
                    // Fall through to reunion tail.
                }

                // Soldier branch.
                ProfileRank::Soldier => {
                    // Only if we have an antagonist (the officer who sent
                    // us out) and the encountered actor is an unreported soldier.
                    let has_antagonist = self.base.antagonist.is_some();
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

                        // Branch on the encountered actor's substate.
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
                                self.return_to_duty_default(sim, ctx, tick);
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
                                // The original game clears alerts from nearby target seekers
                                // synchronously before Say(FOUND_CHARLY).
                                // Preserve that statement boundary: a
                                // rejected speech can immediately dispatch
                                // EVENT_MYTALK_1 and returning to duty, whose
                                // patrol-chief visibility query must not
                                // overtake the earlier Charly-seeker sweep.
                                self.base.outbox.reentrant.owner_work.push(
                                    AiOwnerWork::ActorEffects(std::mem::take(
                                        &mut self.base.outbox.actor,
                                    )),
                                );
                                self.base
                                    .say_with_flags(Remark::FoundCharly, SpeechFlags::MYTALK_1);
                                self.base.outbox.reentrant.owner_work.push(
                                    AiOwnerWork::ResumeSendCharlyAfterSpeech {
                                        charly: charly.get(),
                                    },
                                );
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
        self.base.set_checkpoint_charly(None);

        // Branch on synchronize-index / sync-charly / macro state.
        let no_sync = self.base.synchronize_index == u16::MAX
            || self.base.synchronize_charly.is_none()
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
                    target: self
                        .base
                        .synchronize_charly
                        .expect("synchronization registration requires a friend")
                        .get(),
                    actor: self.base.me,
                },
            );
            self.set_state_with_timer(AiState::Default, Substate::DefaultSynchronizing, 20, ctx);
        }
    }

    // -----------------------------------------------------------------------
    // Unexpected-event dispatch
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
            // missing-PC search whenever the unexpected-event dispatcher
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
                return self.on_unexpected_out_of_view(
                    stimulus,
                    global,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            StimulusType::EventCouldntReachPoint => {
                return self.on_unexpected_couldnt_reach_point(
                    stimulus,
                    global,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                return self.on_unexpected_fit_again(ThinkEnv {
                    sim,
                    ctx,
                    tick,
                    grid,
                });
            }

            StimulusType::EventQuitSwordfight => {
                // Only react if in a real swordfight substate (not
                // approach/run).
                if self.base.current_substate.is_real_swordfight() {
                    // In Merry Man Forest, try to flee via
                    // forest retreat. Only proceed with the normal
                    // quit transition if NOT forest or if flee failed.
                    if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global)
                    {
                        self.set_state(AiState::Attacking, Substate::AttackingQuittingSwordfight);
                        // EVENT_QUIT_SWORDFIGHT calls both reciprocal Update*
                        // setters.
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
                    self.base.outbox.reentrant.owner_work.push(
                        crate::ai::AiOwnerWork::ConsiderToBeginParade {
                            attacker: attacker.get(),
                        },
                    );
                }
            }

            StimulusType::EventSeesSoldier => {
                return self.on_unexpected_sees_soldier(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            StimulusType::CallAlert => {
                return self.on_unexpected_call_alert(
                    stimulus,
                    global,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                self.base.antagonist = Some(officer);
                self.set_state(AiState::Seeking, Substate::SeekingCharlySentToOfficer);
                self.base.set_emoticon(EmoticonType::None);
                self.base.launch_timer(30, ctx.frame);
                self.reported_to_officer = true;
                return true;
            }

            // Officer hails a soldier, never a civilian. CALL_HEY is only dispatched from
            // `SeekingOfficerCallSoldier` with the officer as sender.
            // Soldier accepts the call only if the new task priority
            // outranks the current one.
            StimulusType::CallHey => {
                return self.on_unexpected_call_hey(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                return self.on_unexpected_good_strike(ThinkEnv {
                    sim,
                    ctx,
                    tick,
                    grid,
                });
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
                return self.on_unexpected_sees_beggar(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            StimulusType::EventEnemyNear => {
                tracing::trace!(
                    me = self.base.me,
                    frame = ctx.frame,
                    substate = ?self.base.current_substate,
                    "EventEnemyNear received"
                );
                // Original-game unexpected-event handling,
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
                        self.base.primary_target = Some(enemy);
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
                return self.on_unexpected_after_script_go_on(
                    global,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            StimulusType::EventObjectAway => {
                // Dispatch on object type. `StolenObject` carries the
                // object handle but not its type; we match PURSE/COIN by
                // checking whether the stolen object is tracked as
                // money-of-interest (`interesting_object` or appears in
                // `other_seen_money`). Anything else — including the ALE
                // branch and the default return-to-duty fallback — falls
                // through to `return_to_duty`.
                if let StimulusInfo::Stolen(stolen) = stimulus.info {
                    let obj = stolen.object;
                    let thief = stolen.thief;
                    let is_money_of_interest = self.base.interesting_object == Some(obj)
                        || self.other_seen_money.contains(&obj.get());
                    if is_money_of_interest {
                        self.stolen_money_standard_procedure(thief.get(), ctx, tick);
                    } else {
                        self.return_to_duty_default(sim, ctx, tick);
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
            // the PC-wait state and bumps an X-mark emoticon; if already
            // waiting for the PC we acknowledge silently.
            StimulusType::CallMrOfficerIAmBack => {
                return self.on_unexpected_call_mr_officer_iam_back(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            // A charly the chief was tracking just walked back into view.
            // Clear the checkpoint and watch them resurrect; outside the
            // eligible substate set the charly memory still gets cleared
            // (default arm).
            StimulusType::CallCharlyIsBack => {
                return self.on_unexpected_call_charly_is_back(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                self.base.friend_in_trouble = Some(friend);
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
                self.base.antagonist = Some(officer);
                self.forget_all_nearby_coins(ctx);
                self.set_state(
                    AiState::Wondering,
                    Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl,
                );
                // Timer launch with a duration of 300 + (rand() % 32).
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
    // Alerting-event dispatch
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
                return self.on_alerting_view(
                    stimulus,
                    global,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                return self.on_alerting_arrow_launched(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                    // The shipped game does not reject friendly targets or
                    // forbidden attacks at this point; it still
                    // enters the swordfight. Do not reject these cases here,
                    // or swordfight entry can
                    // attach opponents while this AI stays in its old
                    // state.
                    let allowed_to_attack = self.is_allowed_to_attack(enemy.get(), ctx, tick);
                    if !allowed_to_attack {
                        tracing::warn!(
                            me = self.base.me,
                            enemy = enemy.get(),
                            "EVENT_ENTER_SWORDFIGHT target fails attack eligibility; preserving game behavior and entering anyway"
                        );
                    }
                    self.base.primary_target = Some(enemy);
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
                            self.event_sees_body_standard_procedure(body.get(), ctx, tick, grid);
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
                            self.event_sees_object_standard_procedure(obj.get(), ctx, tick);
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
                return self.on_alerting_got_hit(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
            }

            StimulusType::EventApple => {
                return self.on_alerting_apple(
                    stimulus,
                    ThinkEnv {
                        sim,
                        ctx,
                        tick,
                        grid,
                    },
                );
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
                    self.base.friend_in_trouble = Some(combat.actor_npc);
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
                    self.event_view_standard_procedure(sim, enemy.get(), global, ctx, tick, grid);
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
    // Standard alerting-event procedures
    // -----------------------------------------------------------------------

    /// React to seeing an enemy.
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
            primary_target = ?self.base.primary_target,
            frame = ctx.frame,
            "event_view_standard_procedure: ENTRY"
        );
        if !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
            return;
        }
        self.current_task_priority = self.new_task_priority;
        // Seeing a real enemy supersedes an in-flight curiosity route.
        self.investigating_distraction = false;

        // Royalist-camp early returns. These guards are NOT hoisted into
        // the engine-side dispatcher (which only filters on state), so
        // they must live here to avoid green soldiers chasing
        // already-tied / already-guarded targets and archers on
        // unreachable wall-tops.
        let enemy_view = ctx.entity_view(enemy);
        if ctx.is_player_aligned()
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
        if ctx.is_player_aligned()
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
        if let Some(object) = self.base.object_of_desire.take() {
            self.base.forgotten_objects.push(object.get());
        }

        // Resolve a *fresh* enemy position once and use it for the
        // recon report, the friend-alert broadcast, and the run-near
        // destination—the original game re-reads the enemy planning position
        // literally at each call site rather than using the stale
        // the seek position.
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
        if self.pc_missed && self.missed_pc == Some(AiEntityHandle::new(enemy)) {
            self.pc_missed = false;
        }

        // Alert nearby allies at enemy_pos within VIEW_LOOK_THERE_RADIUS, before
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
        // and re-issue the run-to. Skips the stop-all/speech path entirely so
        // the sprint animation chains straight into the engage.
        if ctx.self_action_state == crate::element::ActionState::MovingFast {
            self.set_state(AiState::Attacking, Substate::AttackingReactiontimeRunning);
            self.base.primary_target = Some(AiEntityHandle::new(enemy));
            self.base.outbox.actor.set_focus(enemy);
            self.reinitialize_them_list(ctx, tick);
            // Run near the enemy position, stopping at one-third distance.
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
            // Enemy distance subtracts the actors' literal 3D
            // world positions, stretches world Y by
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
                primary_target = ?self.base.primary_target,
                "event_view_standard_procedure: EXIT (moving-fast)"
            );
            return;
        }

        // Stop and engage
        self.base.stop_all();
        self.base.say(Remark::SeesEnemy);

        self.base.primary_target = Some(AiEntityHandle::new(enemy));
        self.base.outbox.actor.set_focus(enemy);
        self.reinitialize_them_list(ctx, tick);
        // Standard enemy-sighting response
        // does NOT set `EMOTICON_X_MARK` here — the red `!` only
        // appears when the enemy attack begins after the
        // reaction-time window closes.

        // Three-branch dispatch based on distance and below-flag.
        // Maximum-norm enemy distance deliberately bypasses the AI planning position and
        // subtracts the actors' literal world positions. During a door
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
            // Enemy very near — skip the turn and dispatch battle planning
            // immediately. `IAmInTrouble` is called only on this branch
            // (the broader sightings stay quiet).
            self.set_state(AiState::Attacking, Substate::AttackingReactiontime);
            self.i_am_in_trouble(enemy);
            self.battle_decisions(sim, global, ctx, tick, grid);
        } else if self.enemy_seen_below {
            // Archer saw enemy from a wall — no turn, just a short 5-tick
            // reaction to aim the bow.
            self.set_state_with_timer(AiState::Attacking, Substate::AttackingReactiontime, 5, ctx);
        } else {
            // Standard case — turn towards enemy with a 20-tick
            // the timer as the upper bound for the turn animation.
            // `process_turn_orders` handles the snap + anim booking and
            // the live actor coordinator's `tick_actor_animation_for` fires
            // `EventDone` when the animation completes; whichever fires first
            // wins.
            self.set_state(AiState::Attacking, Substate::AttackingReactiontimeTurning);
            // The original game explicitly faces the enemy here: the alert
            // reaction uses fast turning, including when the sequence is deferred
            // behind an attentive-mode transition.
            self.base.face_entity_fast(enemy, ctx);
            self.base.launch_timer(20, ctx.frame);
        }
        tracing::trace!(
            me = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            primary_target = ?self.base.primary_target,
            timer = self.base.when_does_timer_ring,
            "event_view_standard_procedure: EXIT"
        );
    }

    /// React to hearing a noise.
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
        // A later ordinary noise supersedes the extension-only run latch.
        // The Distraction arm below sets it only after all of its admission
        // checks have passed.
        self.investigating_distraction = false;

        if let Some(object) = self.base.object_of_desire.take() {
            self.base.forgotten_objects.push(object.get());
        }

        // The original game's standard hearing response copies the noise origin
        // verbatim into the seek position. A dry arrow impact outside every
        // motion area legitimately retains the `0xffff` layer and null
        // sector installed by trajectory calculation; position facing then uses
        // its null-sector ground projection. Preserve that authored sentinel
        // instead of rejecting it or manufacturing a world layer.
        let origin_position = noise.origin.legacy_position();

        // Noise-event processing passes the noise's origin to the
        // full-position facing path. Preserve its sector projection when
        // available; replay-normalized sector-less origins fall back to the
        // separately recorded noise elevation instead of fake ground zero.
        match noise.noise_type {
            NoiseType::Distraction => {
                if self.base.current_substate == Substate::SeekingHeardstepsPreReactiontime
                    || self.base.current_substate == Substate::SeekingHeardstepsReactiontime
                    || self.base.current_substate.is_take_money()
                    || self.base.current_substate.is_fight_for_money()
                {
                    return;
                }

                let was_default = self.base.current_state == AiState::Default;
                self.base.stop_all();
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin_position);
                self.base.seek_position = origin_position;
                self.investigating_distraction = true;
                self.base.say(Remark::HearsNoise);
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.set_state(AiState::Seeking, Substate::SeekingHeardstepsPreReactiontime);
                if was_default {
                    self.react(parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16, ctx, tick);
                } else {
                    self.base.launch_timer(1, ctx.frame);
                }
            }

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
                    self.base.seek_position = origin_position;
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
                    .update(ReportType::Noise, origin_position);
                self.base.seek_position = origin_position;

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
                    .update(ReportType::Noise, origin_position);
                self.base.seek_position = origin_position;

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
                self.base.seek_position = origin_position;
                self.base.face_noise_origin_with_ctx(noise, ctx);
                self.base.launch_timer(50, ctx.frame);
            }

            NoiseType::Logs | NoiseType::Drawbridge
                if self.base.current_state == AiState::Default =>
            {
                self.base.stop_all();
                self.set_state(AiState::Wondering, Substate::WonderingWatching);
                self.base.seek_position = origin_position;
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

    /// React to seeing a body.
    fn event_sees_body_standard_procedure(
        &mut self,
        body: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // A local flag captures whether this body is the soldier we
        // were tasked to find via a MissedCharly recon report — used
        // twice below to fire the unalert-cascade on the seeker network.
        let body_view = ctx.entity_view(body);
        let body_pos = body_view
            .map(|v| v.position)
            .unwrap_or(self.base.seek_position);
        let b_hey_this_is_charly = self.base.current_state == AiState::Seeking
            && self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly
            && self.base.my_reconnaissance_report.charly == Some(AiEntityHandle::new(body));

        self.base.my_reconnaissance_report.add_seen_body(body);
        // Reporting the body must use the body's
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

        if let Some(object) = self.base.object_of_desire.take() {
            self.base.forgotten_objects.push(object.get());
        }

        // Already on the way to a body? queue for later.
        match self.base.current_substate {
            Substate::SeekingBodyReactiontime
            | Substate::SeekingBody
            | Substate::SeekingNet
            | Substate::SeekingBodyLookingDeadBody
            | Substate::SeekingBodyAwakeningSleeperr => {
                if Some(AiEntityHandle::new(body)) != self.base.detected_body {
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
                        CharlySeekerTarget::Npc(AiEntityHandle::new(body)),
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
        // Look-there broadcast with default radius.
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
        self.base.detected_body = Some(AiEntityHandle::new(body));
        self.base.outbox.actor.set_focus(body);

        self.set_state(AiState::Seeking, Substate::SeekingBodyReactiontime);

        // Turn to look at the body.
        // The original game faces a full position: preserve its 3D projection and
        // its same-direction Waiting/Bored short-circuit.
        self.base.face_position_3d_with_ctx(body_pos, ctx);
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // The post-state-change charly check fires the unalert cascade for
        // non-mid-seek discoveries too. The body we just saw IS charly,
        // so the sweep target is the body handle.
        if b_hey_this_is_charly {
            self.base.outbox.actor.queue_unalert_near_charly_seekers(
                CharlySeekerTarget::Npc(AiEntityHandle::new(body)),
                self.base.antagonist,
            );
        }
        self.react(
            parameters_ai::AI_MAX_DEADBODY_REACTIONTIME as u16,
            ctx,
            tick,
        );
    }

    /// React to seeing an arrow impact.
    fn event_get_arrow_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pos: &Position,
        global: &AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.current_task_priority = task_priority::ENEMY;

        if let Some(object) = self.base.object_of_desire.take() {
            self.base.forgotten_objects.push(object.get());
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
            // Focusing the interesting object is unconditional in the original game.
            // A missing interesting object is meaningful: clearing focus calls
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
        // The original game sets a yellow, music-only alert.
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
                self.base.interesting_object = Some(AiEntityHandle::new(obj));
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

                // Default arm — note macro interruption (preserves a running
                // sequence for resume) instead of the harder stop-all operation,
                // and `React(AI_FIRST_LOOK_TIME)` instead of the
                // rank-dependent fixed-tick timer.
                self.base.break_macro();
                self.base.say(Remark::SeesObject);
                if let Some(view) = ctx.entity_view(obj) {
                    // Keep the position carried by the live object pointer.
                    // The bottle may become inactive while React's timer is
                    // pending, but the original game can still read the referenced position
                    // when that timer fires.
                    self.base.seek_position = view.position;
                    self.base.face_position_at_elevation_with_ctx(
                        view.position,
                        f32::from(view.elevation as u16),
                        ctx,
                    );
                }
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.base.interesting_object = Some(AiEntityHandle::new(obj));
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
        // Original-game position facing reaches the directional turn, which completes
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
                    // The original game's officer alert constructs the nearby route and
                    // consumes the unreachable-point flag before returning, even
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
            ProfileRank::Knight => {
                unreachable!("RANK_KNIGHT is never eligible for tower-guard call-me dispatch")
            }
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
        if let Some(object) = self.base.object_of_desire.take() {
            self.base.forgotten_objects.push(object.get());
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
    // Per-state reachability fallback when the
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
        // Busy-check preamble — lock AI BUSY, mark was_busy, re-fire
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
            // re-picks a target via battle-overview evaluation.
            AiState::Attacking => {
                if ctx.is_swordfighting {
                    self.set_state_with_timer(
                        AiState::Attacking,
                        Substate::AttackingSwordfight,
                        20,
                        ctx,
                    );
                } else {
                    self.get_battle_overview(0, ctx, tick);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

impl EnemyAi {
    fn on_unexpected_out_of_view(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv {
            sim,
            ctx,
            tick,
            grid,
            ..
        } = env;
        if self.base.current_state == AiState::Attacking
            && let StimulusInfo::Human(enemy) = stimulus.info
        {
            // Original compares the stimulus target strictly with
            // the enemy AI's primary target here
            // for this event. The actor's first
            // opponent can legitimately differ while the AI is in a
            // multi-opponent fight, so it must not stand in for that
            // independent AI member.
            let out_of_view_is_primary = Some(enemy) == self.base.primary_target;
            tracing::trace!(
                me = self.base.me,
                frame = ctx.frame,
                substate = ?self.base.current_substate,
                enemy = enemy.get(),
                primary_target = ?self.base.primary_target,
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
                    if out_of_view_is_primary && self.is_detecting_360_degrees(enemy.get(), ctx) {
                        return false;
                    }
                    // The swordfight labels in turn fall through the
                    // moving-combat stare-vector guard before the
                    // shared lost-enemy handler.
                    if self.enemy_is_behind_me(ctx) {
                        return false;
                    }
                    self.out_of_view_seek_handler(sim, enemy.get(), global, ctx, tick, grid);
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
                    if out_of_view_is_primary && self.is_detecting_360_degrees(enemy.get(), ctx) {
                        // Still close — stay in swordfight.
                        return false;
                    }
                    // The original game's any-swordfight-substate case has
                    // no break here. A failed 360-degree check falls
                    // through the same stare-vector guard used by
                    // REACTIONTIME_RUNNING / APPROACH_TO_OBSERVE /
                    // ADVANCING_WITH_SHIELD before reaching the
                    // shared lost-enemy body.
                    if self.enemy_is_behind_me(ctx) {
                        return false;
                    }
                    self.out_of_view_seek_handler(sim, enemy.get(), global, ctx, tick, grid);
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
                    self.out_of_view_seek_handler(sim, enemy.get(), global, ctx, tick, grid);
                }

                // Stationary / combat-posture substates. On
                // EVENT_OUTOFVIEW, forecast the target's
                // destination and either chase (via seek_area) or
                // face + get_battle_overview.
                //
                // `ATTACKING_REACTIONTIME_TURNING` is explicitly
                // excluded and falls to the default reinitialization
                // branch. The running/walking/charging substates are
                // members of the original game's any-swordfight-substate group
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
                    self.out_of_view_seek_handler(sim, enemy.get(), global, ctx, tick, grid);
                }

                // Do-nothing substates.
                Substate::AttackingTooProudToAttackRetire
                | Substate::AttackingTooProudToAttackRetireTurn
                | Substate::AttackingReactiontimeBending => {}

                // Wait-for-avenger substates. Original
                // sweeps around the
                // waiting soldier itself, not the
                // remembered avenger position it is staring at, and
                // takes the plain battle-overview default flags
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
        false
    }

    fn on_unexpected_couldnt_reach_point(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv { sim, ctx, tick, .. } = env;
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
                if stimulus.self_origin == crate::ai::SelfStimulusOrigin::EngineCompletion
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
                // The lift-entry movement during enemy approach reconsideration is
                // followed immediately by timer launch and return
                // immediately afterward. Control then
                // returns through enemy attack to DECISION_FIGHT, whose
                // couldn't-reachpoint arm switches to DECISION_OBSERVE
                // in the failed-reach branch. The failed
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
                            "ladder route-failure target {:?} disappeared",
                            self.base.primary_target
                        )
                    })
                    .position;
                let target = self.required_primary_target("resuming a failed ladder route");
                let avenger_wait_position =
                    tick.avenger_wait_position_for(self.base.primary_target);
                self.base.couldnt_reachpoint = true;
                if avenger_wait_position.is_some() {
                    self.resume_reconsider_enemy_approach_after_go_near(
                        target_position,
                        avenger_wait_position,
                        ctx,
                    );
                    // The original game constructs and settles this roof approach
                    // before DECISION_OBSERVE returns. Route the typed
                    // actor effects through the existing synchronous
                    // owner boundary so its actual verdict is visible
                    // to this frame's decision-tick completion.
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
                    // Approach movement fails synchronously too in this no-roof
                    // case, so the following source tail installs
                    // observation approach/timer 50 while retaining the
                    // failure latch for tick completion's generic overview.
                    self.resume_battle_observe_after_go_near(
                        target.get(),
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
                // DECISION_OBSERVE's approach. The original game constructs the
                // route inside that statement and tests
                // unreachable-point flag immediately after the state change;
                // Rust can only discover a local Move failure after
                // the typed tail has entered the observation approach.
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
                                "observe route-failure target {:?} disappeared",
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
        false
    }

    fn on_unexpected_fit_again(&mut self, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        // Recovered from unconsciousness.
        //
        // Engine-facing calls share the owner-work FIFO with
        // state changes so their exact decision-tick order survives the
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
            self.return_to_duty_default(sim, ctx, tick);
        } else {
            self.set_state(AiState::Sleeping, Substate::SleepingAwakening);
            self.base
                .outbox
                .reentrant
                .owner_work
                .push(crate::ai::AiOwnerWork::LaunchTimer {
                    frames: parameters_ai::AI_WAKEUP_IDLING_TIME as u32,
                    current_frame: ctx.frame,
                });
            self.base
                .outbox
                .reentrant
                .owner_work
                .push(crate::ai::AiOwnerWork::SetEyeStatus(
                    crate::element::EyeStatus::LookForward,
                ));
        }
        false
    }

    fn on_unexpected_sees_soldier(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, tick, .. } = env;
        // EVENT_SEES_SOLDIER: soldier-spotting-fellow-soldier →
        // "go tell the officer" / "call this soldier over"
        // coordination flow.
        let StimulusInfo::Human(antagonist) = stimulus.info else {
            return false;
        };

        // State/substate reaction gate.
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

        self.base.antagonist = Some(antagonist);
        let antagonist_cs = tick
            .camp_soldiers
            .iter()
            .find(|cs| cs.handle == antagonist.get());

        match self.get_rank() {
            ProfileRank::Soldier => {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::RequestAlert {
                        target: antagonist.get(),
                        caller: self.base.me,
                        continuation: crate::ai::AlertContinuation::SoldierSawOfficer,
                    });
            }
            ProfileRank::Officer => {
                // Officer sees soldier → assert that the seen
                // target is a soldier, gate on
                // soldier-call eligibility, then face + transition
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
                    self.face_npc(antagonist.get(), ctx);
                    // Transition to
                    // SUBSTATE_SEEKING_OFFICER_CALL_SOLDIER — the
                    // EventDone arm of that substate sends
                    // CALL_HEY and launches the soldier-wait
                    // handshake.
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerCallSoldier);
                    // Remove all FRIEND detectables — committed to
                    // this soldier, drop the rest of the friend
                    // list so further EVENT_SEES_SOLDIER calls
                    // don't pre-empt.
                    self.base
                        .outbox
                        .actor
                        .delete_detectable_type(crate::element::DetectableType::Friend);
                }
            }
            ProfileRank::Knight | ProfileRank::None => {
                // Knights never reach EVENT_SEES_SOLDIER in the
                // patrol-coordination flow.
            }
        }
        false
    }

    fn on_unexpected_call_alert(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv {
            ctx, tick, grid, ..
        } = env;
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
                        self.set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                        self.base.face_position(hint.seek_point);
                        self.react(parameters_ai::AI_MAX_ALERT_REACTIONTIME as u16, ctx, tick);
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
                // The original game assigns the antagonist before deciding whether the
                // caller can be heard. A rejected civilian report therefore
                // still replaces the actor tracked by the current behavior.
                self.base.antagonist = Some(civilian);
                if caller.is_civilian() {
                    if self.base.current_state != AiState::Default {
                        return false;
                    }
                    // The original game's civilian alert branch uses the actor's
                    // actor halt directly, not AI stop-all. The actor work
                    // must be interrupted before the state callback, while an
                    // in-flight waypoint macro and its macro timer survive.
                    // This follows the CALL_ALERT civilian branch.
                    self.base.outbox.actor.queue_halt();
                    self.base.face_entity(civilian, ctx);
                    self.set_state(AiState::Seeking, Substate::SeekingWaitForAlertingCivilian);
                    self.base.launch_timer(20, ctx.frame);
                    self.base
                        .set_transient_emoticon(EmoticonType::QuestionMark, 20, ctx.frame);
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
                        if !react || !self.answer_question(Question::HasTheNewTaskPriority, ctx) {
                            return false;
                        }
                        assert_eq!(
                            caller.rank,
                            ProfileRank::Officer,
                            "soldier CALL_ALERT caller must be an officer"
                        );
                        // Original's soldier-from-officer CALL_ALERT arm calls
                        // halts the actor directly, not all AI activity. Halting
                        // interrupts actor work but leaves an in-flight waypoint
                        // macro and its macro timer intact.
                        self.base.outbox.actor.queue_halt();
                        self.current_task_priority = self.new_task_priority;
                        self.gather_position_instructed = false;
                        self.base.friends_are_alerted = true;
                        self.officers_position = caller.position;
                        self.base.face_position_3d_with_ctx(caller.position, ctx);
                        self.set_state(AiState::Seeking, Substate::SeekingGroupCalledByOfficer);
                        self.base.launch_timer(20, ctx.frame);
                        self.base
                            .set_transient_emoticon(EmoticonType::QuestionMark, 20, ctx.frame);
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
                        // halts the actor directly. In particular, it does
                        // not route through AI stop-all and must not break an
                        // in-flight waypoint macro or its macro timer.
                        self.base.outbox.actor.queue_halt();
                        self.base.friends_are_alerted = true;
                        self.base.face_entity(civilian, ctx);
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForAlertingSoldier,
                        );
                        self.base.launch_timer(20, ctx.frame);
                        self.base
                            .set_transient_emoticon(EmoticonType::QuestionMark, 20, ctx.frame);
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
        false
    }

    fn on_unexpected_call_hey(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, .. } = env;
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
        self.base.antagonist = Some(officer);

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
        self.set_state_with_timer(
            AiState::Seeking,
            Substate::SeekingSoldierCalledByOfficer,
            20,
            ctx,
        );
        self.base
            .set_transient_emoticon(EmoticonType::QuestionMark, 20, ctx.frame);
        true
    }

    fn on_unexpected_good_strike(&mut self, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, .. } = env;
        let will_say = self.base.current_substate == Substate::AttackingSwordfightSpecialStrike;
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
        false
    }

    fn on_unexpected_sees_beggar(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, .. } = env;
        // When in a seek-area substate, queue the beggar for later
        // identification (approach → identify1 → identify2).
        if let StimulusInfo::Human(beggar) = stimulus.info
            && self.base.current_substate.is_seek_area()
        {
            if Some(beggar) != self.beggar_to_examine {
                tracing::debug!(
                    beggar = beggar.get(),
                    substate = ?self.base.current_substate,
                    "EventSeesBeggar: queued beggar for identification"
                );
                // Queue beggar for control during seek_next_point(sim, ).
                // Stores the beggar's actual position via the
                // antagonist's position. We read it from the
                // `ctx.antagonist` snapshot populated by the engine
                // when it dispatched this stimulus.
                self.beggars_to_control.push(beggar.get());
                let beggar_pos = ctx
                    .antagonist
                    .as_ref()
                    .map(|a| a.position)
                    .unwrap_or(self.base.seek_position);
                self.positions_of_beggars_to_control.push(beggar_pos);
                self.base
                    .set_transient_emoticon(EmoticonType::QuestionMark, 20, 0);
            }

            // Remove this beggar's DETECTABLE_BEGGAR entry from every NPC.
            // is outside the original game's examined-beggar inequality
            // queueing guard. A repeated view while approaching the
            // claimed beggar must therefore still scrub every NPC's
            // BEGGAR list synchronously through the engine drain.
            self.base.outbox.actor.delete_beggar_for_all_npc.push(
                ctx.entity_id(beggar).unwrap_or_else(|| {
                    panic!("EventSeesBeggar target {beggar} has no typed live entity view")
                }),
            );
        }
        false
    }

    fn on_unexpected_after_script_go_on(
        &mut self,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv {
            sim,
            ctx,
            tick,
            grid,
            ..
        } = env;
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
                self.return_to_duty_default(sim, ctx, tick);
            }
            return false;
        }
        false
    }

    fn on_unexpected_call_mr_officer_iam_back(
        &mut self,
        stimulus: &Stimulus,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv { ctx, .. } = env;
        let StimulusInfo::Human(soldier) = stimulus.info else {
            return false;
        };
        self.base.antagonist = Some(soldier);

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
        true
    }

    fn on_unexpected_call_charly_is_back(
        &mut self,
        stimulus: &Stimulus,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv { ctx, .. } = env;
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
            if self.base.my_reconnaissance_report.charly == Some(charly) {
                self.base.set_checkpoint_charly(None);
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
            self.base.set_checkpoint_charly(None);
        }
        false
    }

    fn on_alerting_view(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> bool {
        let ThinkEnv {
            sim,
            ctx,
            tick,
            grid,
            ..
        } = env;
        if let StimulusInfo::Human(enemy) = stimulus.info {
            match self.base.current_state {
                AiState::Sleeping => {} // ignore (should not happen)
                AiState::Wondering | AiState::Default | AiState::Seeking => {
                    if !self
                        .dispatch_stimulus_to_whole_patrol(sim, stimulus, global, ctx, tick, grid)
                    {
                        self.event_view_standard_procedure(
                            sim,
                            enemy.get(),
                            global,
                            ctx,
                            tick,
                            grid,
                        );
                    }
                }
                AiState::Menacing => {
                    if Some(crate::entity_id::PcId(enemy.get())) != self.guarded_pc {
                        self.event_view_standard_procedure(
                            sim,
                            enemy.get(),
                            global,
                            ctx,
                            tick,
                            grid,
                        );
                    }
                }
                AiState::Fleeing => {
                    // Ignore EVENT_VIEW while fleeing to leave the
                    // map (merry man flee) or while running back
                    // for arrow reserves.
                    if self.base.current_substate == Substate::FleeingMerryManRunToLeaveMap
                        || self.base.current_substate == Substate::FleeingMerryManLeaveMap
                        || self.base.current_substate == Substate::FleeingRunForArrowReserves
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
                            // The original game's enemy list is unique: the
                            // preceding VIEW may already have rebuilt
                            // this target into the final visible set.
                            if !self.list_them.contains(&enemy.get()) {
                                self.list_them.push(enemy.get());
                            }
                        }

                        Substate::AttackingArcherWaitOnArcheryPath
                        | Substate::AttackingArcherWaitOnBendPoint
                        | Substate::AttackingArcherWaitOnArcheryPathBending => {
                            // Archer waiting on firing point —
                            // rebuild list, re-eval elevation,
                            // re-run battle planning.
                            self.reinitialize_them_list(ctx, tick);
                            self.enemy_seen_below = enemy_is_below_me(
                                ctx,
                                tick.owner_live_position.or(Some(ctx.position)),
                                tick.enemy_detectable_live_world_position(enemy.get())
                                    .or_else(|| {
                                        ctx.entity_view(enemy)
                                            .map(|view| view.detection_position_world)
                                    }),
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
                                    sim,
                                    enemy.get(),
                                    global,
                                    ctx,
                                    tick,
                                    grid,
                                );
                            }
                        }

                        // Indoor door-fight — escalate to
                        // building-wide alert.
                        Substate::AttackingDoorFightDelay | Substate::AttackingDoorFightLeaving
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
                            // battle planning.
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
        false
    }

    fn on_alerting_arrow_launched(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, .. } = env;
        // A shield bearer whose current substate says "I am
        // holding / advancing under a shield" slams the shield up
        // against the incoming arrow and pivots to face the
        // shooter.
        if let StimulusInfo::Human(shooter) = stimulus.info {
            // Protecting with a shield: protection is already in
            // WAITING_SHIELD?  false : true — i.e., only re-raise
            // if we're still mid-animation.
            // Advancing / RunningToPhalanx: always protect.
            let b_protect = match self.base.current_substate {
                Substate::AttackingProtectingWithShield => ctx
                    .entity_view(self.base.me)
                    .map(|v| v.current_animation != crate::order::OrderType::WaitingShield)
                    .unwrap_or(false),
                Substate::AttackingAdvancingWithShield | Substate::AttackingRunningToPhalanx => {
                    true
                }
                _ => false,
            };

            if b_protect {
                use crate::element::Command;
                use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

                // Remember the shooter.
                self.base.primary_target = Some(shooter);

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

                // Original immediately repeats state assignment and
                // shield updates after the synchronous instant-raise
                // launch, then Focuses the shooter. Close that actor
                // prefix so the trailing Focus cannot overtake it at
                // the deferred owner boundary.
                self.base.outbox.actor.raise_shield_immediately = true;
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                        &mut self.base.outbox.actor,
                    )));

                self.base.outbox.actor.set_focus(shooter);

                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingProtectingWithShield,
                    15,
                    ctx,
                );
            }
        }
        false
    }

    fn on_alerting_got_hit(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv {
            ctx, tick, grid, ..
        } = env;
        // Three arms: (1) swordfighting → add opponent if
        // cross-camp & not already engaged; (2) MenacingPcInComa →
        // return-to-PC transition with no opponent
        // ENTER_SWORDFIGHT sequence; (3) generic else → stop_all
        // + non-human filter + brawl-friend-in-trouble +
        // attack_enemy plus dead-or-unconscious view-status assignment.
        // The original game checks whether the human is swordfighting,
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
                let attacker_is_hostile = ctx.is_hostile_with(attacker_view.camp);
                if attacker_is_hostile {
                    let already_opponent = self
                        .find_fighter(self.base.me, tick)
                        .unwrap_or_else(|| {
                            panic!(
                                "soldier {} EVENT_GOTHIT requires self fighter snapshot",
                                self.base.me
                            )
                        })
                        .has_as_opponent(attacker.get());
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
                self.base.primary_target = Some(attacker);
                self.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::RaiseSword);
                self.base.outbox.actor.enter_swordfight_jump_line = None;
                // The original game sets element direction here, not
                // AI facing. The hit animation
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
                let attacker_is_soldier = attacker_view.map(|v| v.is_soldier()).unwrap_or(false);
                let attacker_in_brawl = attacker_view
                    .map(|v| v.ai_substate.is_fight_for_money())
                    .unwrap_or(false);
                if attacker_is_soldier {
                    if attacker_in_brawl {
                        // Brawl-friend hit me — capture as
                        // friend_in_trouble, transition to
                        // WonderingBrawlGotHit, clear emoticon.
                        self.base.friend_in_trouble = Some(attacker);
                        self.set_state(AiState::Wondering, Substate::WonderingBrawlGotHit);
                        self.base.set_emoticon(EmoticonType::None);
                    }
                    // Soldier-attacker in non-brawl substate:
                    // falls through the empty switch — no
                    // primary_target / attack_enemy update; only
                    // view-status assignment below applies.
                } else {
                    // Non-soldier human attacker — retarget and
                    // attack.
                    self.base.primary_target = Some(attacker);
                    self.attack_enemy(attacker.get(), ctx, tick, grid);
                }
                // Dead-or-unconscious view-status assignment
                // applies whenever the attacker info was human,
                // regardless of which sub-arm fired.
                // Keep this on the owner FIFO: in the Original this
                // statement is the tail of EVENT_GOTHIT, after every
                // stop-all / enemy-attack actor work has completed.
                // Close the actor prefix explicitly: enemy attack can
                // reach another stop-all request whose deferred halt notification
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
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::SetEyeStatus(
                        crate::element::EyeStatus::DieOrGetUnconscious,
                    ));
            } else {
                // Non-human stimulus info — clear primary_target.
                self.base.primary_target = None;
            }
        }
        false
    }

    fn on_alerting_apple(&mut self, stimulus: &Stimulus, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { sim, ctx, .. } = env;
        let in_swordfight_state = self.base.current_substate.is_any_swordfight();
        let may_interrupt = sim.config().item_gameplay.apple_combat_interrupt;
        if (!in_swordfight_state || may_interrupt)
            && let StimulusInfo::Position(ref pos) = stimulus.info
        {
            self.base.stop_all();
            // Original-game soldier apple-alert handling
            // rejects every swordfight substate. The optional rule
            // deliberately breaks the reciprocal fight before the
            // apple daze takes ownership of the actor.
            if may_interrupt && ctx.is_swordfighting {
                self.base.outbox.actor.quit_swordfight = true;
            }
            self.base.seek_position = *pos;
            self.set_state(AiState::Wondering, Substate::WonderingAppleSauceInTheVisor);
            // Spawn a
            // `RHTITBIT_WEAK_STUNNED` titbit at
            // the computed stars-effect point if one doesn't already exist on
            // this NPC.  The AI can't touch the titbit manager,
            // so we lean on `EngineInner::sync_apple_sauce_titbits`
            // which runs every frame, scans for any NPC in
            // `WonderingAppleSauceInTheVisor`, and calls
            // `add_weak_stunned` — which internally runs
            // `TitbitExists` guard + `compute_stars_point`.  The
            // effect is same-frame (AI ticks before `sync_titbits`
            // in `perform_hourglass_inner`).
            // Apple hits visor, vision is restored gradually via
            // Gradually reopen eyes (view cone grows from 5 back to
            // standard radius).
            self.base.outbox.actor.slowly_open_eyes = true;
            self.base.launch_timer(60, ctx.frame);
        }
        false
    }
}
use super::ThinkEnv;
