//! `EnemyAi::think_expected_event` — the substate state machine.
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the giant per-substate match
//! manageable. Lives in a separate `impl EnemyAi` block; child modules
//! see the parent's private fields and helpers.

mod attacking;

use crate::ai::*;
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

use super::util::{
    ai_max_norm_distance, ai_max_norm_distance_world, ai_square_distance, resolve_seek_point_id,
    vec_to_sector,
};
use super::{
    AlertSoldiersFailureContinuation, EnemyAi, PrimaryTargetFlags, ProfileRank, SeekFlags,
    ThinkEnv, UNDEFINED_DIRECTION, archer, combat, task_priority,
};

fn approaching_new_enemy_is_close_enough(
    target: &Position,
    target_elevation: f32,
    owner: &Position,
    owner_elevation: f32,
    sword_range: u16,
) -> bool {
    let range_with_margin = u32::from(sword_range) + 10;
    let range_squared = range_with_margin.wrapping_mul(range_with_margin);
    ai_square_distance(target, target_elevation, owner, owner_elevation) < range_squared as f32
}

impl EnemyAi {
    // One dispatcher preserves the numeric Substate machine while the
    // implementations are owned by coherent state families. See
    // the original game's expected-event handling.
    pub(super) fn think_expected_event(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        debug_assert_eq!(
            self.base.current_substate.ai_state_family(),
            Some(self.base.current_state),
            "EnemyAi expected-event dispatch received mismatched state/substate: {:?}/{:?}",
            self.base.current_state,
            self.base.current_substate
        );

        Ok(match self.base.current_state {
            AiState::Sleeping => self.think_expected_sleeping_event(stimulus, env.ctx),
            AiState::Default => self.think_expected_default_event(stimulus, global, env)?,
            AiState::Wondering => self.think_expected_wondering_event(stimulus, global, env)?,
            AiState::Seeking => self.think_expected_seeking_event(stimulus, global, env)?,
            AiState::Attacking => self.think_expected_attacking_event(stimulus, global, env)?,
            AiState::Menacing => self.think_expected_menacing_event(stimulus, env)?,
            AiState::Fleeing => self.think_expected_fleeing_event(stimulus, global, env)?,
        })
    }

    fn think_expected_sleeping_event(&mut self, stimulus: &Stimulus, ctx: &AiContext) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        if let Substate::SleepingAwakening = self.base.current_substate
            && matches!(
                stimulus_type,
                StimulusType::EventDone | StimulusType::EventTimer
            )
        {
            if let Some(alert_path_id) = self.base.alert_path_id
                && !self.changed_to_alert_path
            {
                self.changed_to_alert_path = true;
                // Rebuild the patrol path from the alert-path
                // hiking path index.
                let hiking_paths = &ctx.hiking_paths;
                self.base.patrol_path = crate::ai::PatrolPath::new(alert_path_id, hiking_paths);
                self.base.has_patrol_path = self.base.patrol_path.is_some();
            }
            self.base.set_emoticon(EmoticonType::QuestionMark);
            self.set_state_with_timer(AiState::Wondering, Substate::WonderingLooking1, 30, ctx);
        }
        false
    }

    fn think_expected_default_event(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::DefaultGotoPost => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // The original game's common handler performs directional facing followed
                    // by state changes. For an enemy soldier that second
                    // call must reach EnemyAi::set_state so returning to
                    // Default queues LeaveAttentiveMode and ALERT_GREEN.
                    self.base
                        .face_direction(self.base.initial_view_direction, ctx);
                    self.set_state(AiState::Default, Substate::DefaultGotoPostTurn);
                    return Ok(true);
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultGotoPostTurn => {
                if stimulus_type == StimulusType::EventDone {
                    if self.base.likes_to_sit_around {
                        self.base.outbox.actor.posture = Some(crate::element::Posture::Sitting);
                    } else if self.base.special_action {
                        self.base.outbox.actor.posture = Some(crate::element::Posture::Leisure);
                    }
                    self.set_state(AiState::Default, Substate::DefaultOnPost);
                    let bored = self.base.get_bored_time(sim, ctx);
                    self.base.launch_timer(bored as u32, ctx.frame);
                    return Ok(true);
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultGotoRoute
            | Substate::DefaultGotoRouteTurn
            | Substate::DefaultOnPost
            | Substate::DefaultEnroute
            | Substate::DefaultInMacro
            | Substate::DefaultInMacroWaitingForDone => {
                // `think_expected_event_common_stuff` requests the specialized
                // `default_bored_standard_procedure` on timer expiry
                // during `DefaultOnPost`. Run it before delegating so
                // the subclass override takes effect; if it transitions
                // state we short-circuit, otherwise fall through to the
                // base timer.
                if self.base.current_substate == Substate::DefaultOnPost
                    && stimulus_type == StimulusType::EventTimer
                    && self.default_bored_standard_procedure(sim, ctx)
                {
                    return Ok(true);
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultOnPostLookingSidewards => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Default, Substate::DefaultOnPost);
                    let bored = self.base.get_bored_time(sim, ctx);
                    tracing::trace!(
                        me = self.base.me,
                        bored,
                        frame = ctx.frame,
                        "look-sidewards done; relaunching bored timer"
                    );
                    self.base.launch_timer(bored as u32, ctx.frame);
                }
            }

            Substate::DefaultLookingOfficerForAdvice => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty_default(env)?;
                }
            }

            Substate::DefaultLookingShadow => {
                // Keep watching as long as the shadow is still somewhat
                // visible. The engine updates `max_visibility` each
                // detection tick; if it drops to 0 the target is fully
                // hidden again.
                if stimulus_type == StimulusType::EventTimer {
                    if self.base.max_visibility > 0 {
                        // Target still partially visible — keep looking
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.return_to_duty_default(env)?;
                    }
                }
            }

            // ============ PATROL ENROUTE ============
            Substate::DefaultPatrolEnroute | Substate::DefaultPatrolEnrouteRunning => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Reached our position in the formation — face
                    // patrol direction.  Only issue the `face_to` when
                    // the current facing differs, otherwise the no-op
                    // turn re-triggers a bogus `EventDone` through the
                    // sequence manager.
                    if self.base.patrol_direction != ctx.direction {
                        self.base.face_direction(self.base.patrol_direction, ctx);
                    }
                    self.set_state_with_timer(
                        AiState::Default,
                        Substate::DefaultPatrolEnrouteWaiting,
                        200,
                        ctx,
                    );
                }
            }

            Substate::DefaultPatrolEnrouteWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    // Check patrol chief's AI state (cached by engine each patrol tick).
                    // If chief is in Default or Wondering, keep waiting for next
                    // coordinate call. Otherwise the chief is in trouble — abandon.
                    match tick.patrol_chief_state {
                        AiState::Default | AiState::Wondering => {
                            self.base.launch_timer(200, ctx.frame);
                        }
                        _ => {
                            // Chief is in combat or otherwise unavailable
                            self.return_to_duty_default(env)?;
                        }
                    }
                }
            }

            Substate::DefaultGotoChief => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if let Some(patrol_chief) = self.base.patrol_chief {
                        // The original game uses the element-facing variant
                        // facing the patrol chief, which includes the chief's
                        // truncated elevation in the projection.
                        self.base.face_entity(patrol_chief.index(), ctx);
                        self.set_state_with_timer(
                            AiState::Default,
                            Substate::DefaultPatrolEnrouteWaiting,
                            200,
                            ctx,
                        );
                    } else {
                        // Lost patrol chief — retry
                        self.return_to_duty_default(env)?;
                    }
                }
            }

            // ============ PATROL CHIEF RETURN ============
            Substate::DefaultPatrolChiefReturnToPatrol => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.return_to_duty_default(env)?;
                }
            }

            // ============ WONDERING ============
            Substate::DefaultScriptDriven => {}

            // Soldier keeps scanning for Charly while on duty.
            // Random sidewards look, sorrow-level accumulation, and
            // periodic re-seeking.
            Substate::DefaultLookingForCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    let rand_sorrow =
                        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CharlySorrow, 0..5000)
                            as u16;
                    if rand_sorrow < self.base.sorrow_level + 10 {
                        self.set_state(
                            AiState::Default,
                            Substate::DefaultLookingSidewardsForCharly,
                        );
                        self.base.outbox.actor.look_sidewards = Some(
                            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CharlySorrow, 0..2)
                                != 0
                            {
                                LookDirection::LeftRight
                            } else {
                                LookDirection::RightLeft
                            },
                        );
                    }
                    self.base.sorrow_level = self
                        .base
                        .sorrow_level
                        .saturating_add(self.base.delta_sorrow_level);
                    if self.base.sorrow_level > 1000 {
                        self.base.sorrow_level = 0;
                        self.search_charly(env, global)
                            .map_err(|duty| duty.then(crate::ai::DutyTail::SearchCharlyTimer))?;
                    }
                    self.base
                        .launch_timer(parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32, ctx.frame);
                }
            }

            // Done sweeping eyes, back to baseline looking for Charly.
            Substate::DefaultLookingSidewardsForCharly => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state_with_timer(
                        AiState::Default,
                        Substate::DefaultLookingForCharly,
                        10,
                        ctx,
                    );
                }
            }

            // Reacted to detecting Charly; either resume macro or
            // return to duty.
            Substate::DefaultDetectedCharly => {
                if stimulus_type == StimulusType::EventTimer {
                    if self.base.macro_in_progress {
                        self.set_state(AiState::Default, Substate::DefaultInMacro);
                        self.base.execute_next_macro_command(sim, ctx);
                    } else {
                        self.return_to_duty_default(env)?;
                    }
                }
            }

            // Synchronize with Charly; if he's gone astray, give up;
            // else wait for a PC synchronization event.
            Substate::DefaultSynchronizing => match stimulus_type {
                StimulusType::EventTimer => {
                    // If `synchronize_charly` is not in STATE_DEFAULT
                    // or is dead, return to duty; else re-arm the
                    // timer.
                    // A vanished (or unset) partner counts as gone.
                    let sync_gone = ctx
                        .entity_view_logged(self.base.synchronize_charly, "synchronize charly")
                        .is_none_or(|v| v.ai_state != AiState::Default || !v.is_able_to_fight);
                    if sync_gone {
                        self.return_to_duty_default(env)?;
                    } else {
                        self.base.launch_timer(20, ctx.frame);
                    }
                }
                StimulusType::EventSyncCharly => {
                    if let crate::ai::StimulusInfo::Index(idx) = stimulus.info
                        && idx == self.base.synchronize_index
                    {
                        // Assertion: `macro_in_progress` is true here.
                        self.set_state(AiState::Default, Substate::DefaultInMacro);
                        self.base.execute_next_macro_command(sim, ctx);
                    }
                }
                _ => {}
            },

            // WonderingLooking3 shares the timer-to-sidewards
            // transition with looking 1/2; the next state is
            // WonderingLooking3Sidewards.
            _ => {}
        }
        Ok(false)
    }

    fn think_expected_wondering_event(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        Ok(match self.base.current_substate {
            Substate::WonderingWatching => self.wondering_watching(env, stimulus_type)?,

            Substate::WonderingLooking1 => self.wondering_looking1(sim, stimulus_type, ctx),

            Substate::WonderingLooking1Sidewards => {
                self.wondering_looking1_sidewards(sim, stimulus_type, ctx)
            }

            Substate::WonderingLooking2 => self.wondering_looking2(sim, stimulus_type, ctx),

            Substate::WonderingLooking2Sidewards => {
                self.wondering_looking2_sidewards(sim, stimulus_type, ctx)
            }

            Substate::WonderingMoneyReactiontime => {
                self.wondering_money_reactiontime(env, stimulus_type)?
            }

            Substate::WonderingApproachingMoney => {
                self.wondering_approaching_money(stimulus_type, ctx, tick)
            }

            Substate::WonderingTakingMoney => self.wondering_taking_money(stimulus_type, ctx),

            Substate::WonderingWatchingForMoreMoney => {
                self.wondering_watching_for_more_money(env, stimulus_type)?
            }

            Substate::WonderingAleReactiontime => {
                self.wondering_ale_reactiontime(env, stimulus_type)?
            }

            Substate::WonderingApproachingAle => self.wondering_approaching_ale(stimulus_type, ctx),

            Substate::WonderingDrinkingAle => self.wondering_drinking_ale(env, stimulus_type)?,

            Substate::WonderingWatchingTowerGuard => {
                self.wondering_watching_tower_guard(env, stimulus_type)?
            }

            Substate::WonderingLooking3 => self.wondering_looking3(sim, stimulus_type),

            Substate::WonderingLooking3Sidewards => {
                self.wondering_looking3_sidewards(env, stimulus_type)?
            }

            Substate::WonderingRunningForMoney => {
                self.wondering_running_for_money(stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlReactiontime => {
                self.wondering_brawl_reactiontime(stimulus_type, ctx)
            }

            Substate::WonderingBrawlApproaching => {
                self.wondering_brawl_approaching(env, stimulus_type)?
            }

            Substate::WonderingBrawlHitting => self.wondering_brawl_hitting(stimulus_type),

            Substate::WonderingBrawlGotHit => {
                self.wondering_brawl_got_hit(stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlRecovering => {
                self.wondering_brawl_recovering(stimulus_type, ctx, tick)?
            }

            Substate::WonderingApproachingToLoot => {
                self.wondering_approaching_to_loot(stimulus_type, ctx)
            }

            Substate::WonderingLooting => self.wondering_looting(env, stimulus_type)?,

            Substate::WonderingAleAway => self.wondering_ale_away(env, stimulus_type)?,

            Substate::WonderingOfficerSeeingBrawl => {
                self.wondering_officer_seeing_brawl(stimulus_type, ctx)
            }

            Substate::WonderingOfficerApproachingBrawl => {
                self.wondering_officer_approaching_brawl(stimulus_type, ctx, tick)?
            }

            Substate::WonderingOfficerFinishingBrawl => {
                self.wondering_officer_finishing_brawl(env, stimulus_type)?
            }

            Substate::WonderingOfficerFinishingBrawlWaiting => {
                self.wondering_officer_finishing_brawl_waiting(env, stimulus_type)?
            }

            Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl => {
                self.wondering_soldier_looking_officer_who_finished_brawl(env, stimulus_type)?
            }

            Substate::WonderingApproachingBrawlVictim => {
                self.wondering_approaching_brawl_victim(stimulus_type)
            }

            Substate::WonderingAwakenBrawlVictim => {
                self.wondering_awaken_brawl_victim(env, stimulus_type)?
            }

            // Attacker returns to another PC after menacing: begin a
            // swordfight.
            _ => false,
        })
    }

    fn wondering_watching(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    fn wondering_looking1(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Wondering, Substate::WonderingLooking1Sidewards);
            // Random LR or RL.
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..2) != 0
                {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                },
            );
            self.base
                .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
        }
        false
    }

    // Sidewards finished: transition to next looking stage,
    // Face (dir + 5) % 16, then launch_timer(30 + rand()&7).
    // Shared body for stages 1 & 2.

    fn wondering_looking1_sidewards(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(AiState::Wondering, Substate::WonderingLooking2);
            let dir = (ctx.direction + 5) & 15;
            self.base.face_direction(dir, ctx);
            self.base.launch_timer(
                30 + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..8),
                ctx.frame,
            );
        }
        false
    }

    fn wondering_looking2(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Wondering, Substate::WonderingLooking2Sidewards);
            // Random LR or RL.
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..2) != 0
                {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                },
            );
            self.base
                .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
        }
        false
    }

    // Same shared body for Looking2Sidewards: transition to
    // Looking3 (NOT return to duty), face (dir + 5) % 16,
    // 30 + rand()&7 timer.

    fn wondering_looking2_sidewards(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(AiState::Wondering, Substate::WonderingLooking3);
            let dir = (ctx.direction + 5) & 15;
            self.base.face_direction(dir, ctx);
            self.base.launch_timer(
                30 + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..8),
                ctx.frame,
            );
        }
        false
    }

    // Money reactiontime:
    // YES branch: clean stale entries, switch state, SUN emoticon
    // (20-tick), say GoldYes, approach, 5-tick timer.
    // NO branch: CLOUD emoticon (50-tick), Say(GoldNo/VipGoldNo),
    // forget nearby coins, return to duty while keeping the emoticon.

    fn wondering_money_reactiontime(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, tick, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            let want_money = self.answer_question(Question::ShallITakeMoney, ctx);
            let obj_pos = ctx
                .entity_view_logged(self.base.interesting_object, "money being reacted to")
                .map(|v| v.position);
            let officer_near = obj_pos.is_some_and(|p| self.is_any_angry_officer_near(p, tick));
            if want_money
                && let Some(obj_pos) = obj_pos
                && !officer_near
            {
                // Drop destroyed entries.
                self.clean_up_list_of_seen_money(ctx);
                self.base.say(Remark::GoldYes);
                self.set_state(AiState::Wondering, Substate::WonderingApproachingMoney);
                self.base
                    .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingApproachingMoney,
                    obj_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
                self.base.launch_timer(5, ctx.frame);
            } else {
                self.base
                    .set_transient_emoticon(EmoticonType::Cloud, 50, ctx.frame);
                if self.is_vip {
                    self.base.say(Remark::VipGoldNo);
                } else {
                    self.base.say(Remark::GoldNo);
                }
                // Clear other-seen-money list + forget nearby
                // coins so this NPC doesn't re-trigger the
                // money-want flow this tick.
                self.other_seen_money.clear();
                self.forget_all_nearby_coins(ctx);
                return Err(crate::ai::DutyCall::new(DutyFlags::KEEP_EMOTICON, false));
            }
        }
        Ok(false)
    }

    fn wondering_approaching_money(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Approaching and running for money share event handling.
        // EVENT_TIMER refreshes the race; only
        // EVENT_REACHPOINT starts the Take interaction.
        self.wondering_running_for_money(stimulus_type, ctx, tick)
    }

    fn wondering_taking_money(&mut self, _stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        // Original intentionally ignores the expected-event type here.  The
        // Take sequence normally finishes with EVENT_DONE, but any expected
        // event advances the same completion boundary.
        if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
            self.base.interesting_object = Some(AiEntityHandle::new(coin));
            self.set_state_with_timer(
                AiState::Wondering,
                Substate::WonderingMoneyReactiontime,
                1,
                ctx,
            );
        } else {
            self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
        }
        false
    }

    fn wondering_watching_for_more_money(
        &mut self,
        _env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            return Err(DutyCall {
                tail: crate::ai::DutyTail::MoneyFight {
                    operation: crate::ai::MoneyFightOperation::CollectOrLootAfterLook,
                },
                ..DutyCall::new(DutyFlags::empty(), false)
            });
        }
        Ok(false)
    }

    // Ale reactiontime: if shall-take-ale:
    // stash beer as object_of_desire, transition to
    // ApproachingAle, set SUN emoticon (20-tick), Say(AleYes),
    // Approach, save return point, 20-tick timer. Otherwise
    // CLOUD emoticon (50-tick) + Say(AleNo / VipAleNo) +
    // Return to duty with KEEP_EMOTICON.

    fn wondering_ale_reactiontime(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            if self.answer_question(Question::ShallITakeAle, ctx) {
                assert!(
                    self.base.interesting_object.is_some(),
                    "ale reaction timer requires the retained bottle pointer"
                );
                // The original game reads the interesting object's position even when a
                // different soldier has just consumed and deactivated the
                // bottle. Inactive objects are absent from AiContext, so use
                // the position latched by EventSeesObject in that case.
                let obj_pos = ctx
                    .entity_position(self.base.interesting_object)
                    .unwrap_or(self.base.seek_position);
                self.base.object_of_desire = self.base.interesting_object;
                self.set_state(AiState::Wondering, Substate::WonderingApproachingAle);
                self.base
                    .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                self.base.say(Remark::AleYes);
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingApproachingAle,
                    obj_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
                self.return_to_patrol_point = ctx.position;
                self.base.launch_timer(20, ctx.frame);
            } else {
                self.base
                    .set_transient_emoticon(EmoticonType::Cloud, 50, ctx.frame);
                if self.is_vip {
                    self.base.say(Remark::VipAleNo);
                } else {
                    self.base.say(Remark::AleNo);
                }
                return Err(crate::ai::DutyCall::new(DutyFlags::KEEP_EMOTICON, false));
            }
        }
        Ok(false)
    }

    fn wondering_approaching_ale(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        // The TIMER and REACHPOINT arms both gate on
        // `is_beer_still_available`.  On failure (bottle gone
        // or stolen) both paths face the lost position, flip
        // to THUNDERSTORM, switch to `WonderingAleAway`, and
        // arm a 30-tick recovery timer.  On success the
        // TIMER arm re-arms a 20-tick poll, and the
        // REACHPOINT arm launches the drink-ale sequence and
        // transitions to `WonderingDrinkingAle`.
        match stimulus_type {
            StimulusType::EventTimer => {
                if let Some(lost_pos) = self.is_beer_still_available(ctx) {
                    self.base.face_position_3d_with_ctx(lost_pos, ctx);
                    self.base.set_emoticon(EmoticonType::Thunderstorm);
                    self.set_state_with_timer(
                        AiState::Wondering,
                        Substate::WonderingAleAway,
                        30,
                        ctx,
                    );
                } else {
                    self.base.launch_timer(20, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                if let Some(lost_pos) = self.is_beer_still_available(ctx) {
                    self.base.face_position_3d_with_ctx(lost_pos, ctx);
                    self.base.set_emoticon(EmoticonType::Thunderstorm);
                    self.set_state_with_timer(
                        AiState::Wondering,
                        Substate::WonderingAleAway,
                        30,
                        ctx,
                    );
                } else {
                    self.set_state(AiState::Wondering, Substate::WonderingDrinkingAle);
                    // Launch a DrinkAle interaction to trigger
                    // the drinking animation on the ale bottle.
                    if let Some(obj) = self.base.interesting_object {
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(ctx.entity_id(obj).unwrap_or_else(|| {
                            panic!(
                                "ale interaction object handle {obj} has no live typed entity view"
                            )
                        }));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::DrinkAle,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn wondering_drinking_ale(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    fn wondering_watching_tower_guard(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // ============ SEEKING ============

    // -- Seek-area substates --

    fn wondering_looking3(&mut self, sim: &SimulationContext, stimulus_type: StimulusType) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Wondering, Substate::WonderingLooking3Sidewards);
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemyWonderingLook, 0..2) != 0
                {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                },
            );
        }
        false
    }

    // Done sweeping eyes after awakening/wasp sting; return to duty.

    fn wondering_looking3_sidewards(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // Running for money: race rivals on timer, and on reach,
    // take it or look for more.

    fn wondering_running_for_money(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // If another guy is in sight approaching the money,
                // re-stage RunningForMoney and notify any patrol
                // chief; otherwise just re-arm the timer.
                //
                // `there_is_another_guy_in_sight_approaching_to_money`
                // walks same-camp soldiers and checks
                // is_take_money || is_fight_for_money (minus
                // MoneyReactiontime), not self, and
                // `is_detecting_180_degrees`.
                let another_guy_approaching =
                    self.there_is_another_guy_in_sight_approaching_to_money(ctx, tick);
                if another_guy_approaching {
                    // Run to an accessible point within
                    // AI_STOP_BEFORE_MONEY_DISTANCE of the money.
                    if let Some(obj_pos) = ctx.entity_position(self.base.interesting_object) {
                        self.go_near(
                            AiState::Wondering,
                            Substate::WonderingRunningForMoney,
                            obj_pos,
                            parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                            crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::FIND_ACCESSIBLE,
                            ctx,
                        );
                    }
                    // If my patrol chief is an officer whose 180°
                    // detects me, fire EVENT_SEES_BRAWL at them.
                    if let Some(chief_id) = self.base.patrol_chief
                        && let Some(chief_view) = ctx.entity_view(chief_id.index())
                        && chief_view.is_soldier()
                        && chief_view.is_able_to_fight
                        && chief_view.rank == ProfileRank::Officer
                    {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                target: chief_id.index(),
                                stimulus_type: StimulusType::EventSeesBrawl,
                                info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(
                                    self.base.me,
                                )),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            },
                        );
                    }
                } else {
                    self.base.launch_timer(20, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                // If money is still active and within 25 units
                // (maximum norm), take it + notify friends with
                // EventObjectAway; else look for more.
                let obj = self.base.interesting_object;
                let close_enough = ctx
                    .entity_view_logged(obj, "money being approached")
                    .is_some_and(|v| {
                        let dx = (v.position.x - ctx.position.x).abs();
                        let dy = (v.position.y - ctx.position.y).abs();
                        dx.max(dy) < 25.0
                    });
                if let Some(obj) = obj.filter(|_| close_enough) {
                    // Stop actions and launch the Take sequence.
                    self.base.stop_all();
                    use crate::element::Command;
                    use crate::sequence::{Sequence, SequenceElement};
                    let owner = self.base.owner_entity_id;
                    let antagonist = Some(ctx.entity_id(obj).unwrap_or_else(|| {
                        panic!(
                            "money interaction object handle {obj} has no live typed entity view"
                        )
                    }));
                    let mut seq = Sequence::new();
                    seq.append_element(SequenceElement::new_interaction(
                        1,
                        Command::Take,
                        owner,
                        antagonist,
                    ));
                    self.base.outbox.actor.launch_sequences.push(seq);

                    // Notify any same-camp soldier whose substate
                    // is take-money or fight-for-money with
                    // EventObjectAway carrying a StolenObject.
                    let stolen = crate::ai::StolenObject {
                        object: obj,
                        thief: AiEntityHandle::new(self.base.me),
                    };
                    for cs in tick.camp_soldiers.iter() {
                        if cs.handle == self.base.me {
                            continue;
                        }
                        if cs.ai_substate.is_take_money() || cs.ai_substate.is_fight_for_money() {
                            self.base.outbox.reentrant.cross_npc_actions.push(
                                CrossNpcAction::SendStimulus {
                                    target: cs.handle,
                                    stimulus_type: StimulusType::EventObjectAway,
                                    info: crate::ai::StimulusInfo::Stolen(stolen),
                                    fallback_to_sender: None,
                                    to_whole_patrol: false,
                                },
                            );
                        }
                    }

                    self.set_state(AiState::Wondering, Substate::WonderingTakingMoney);
                } else {
                    // Transition to WatchingForMoreMoney + look
                    // sidewards.
                    self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
                    self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                }
            }
            _ => {}
        }
        false
    }

    // Brawl reaction: set mood, approach the friend in
    // trouble, run on timer tick.

    fn wondering_brawl_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Wondering, Substate::WonderingBrawlApproaching);
            self.base.set_emoticon(EmoticonType::Thunderstorm);
            self.base.say(Remark::GoldBrawl);
            //   seek_position = friend_in_trouble.position;
            //   run to within AI_HIT_DISTANCE of seek_position;
            let view = ctx.expect_entity_view(
                self.base.friend_in_trouble,
                "brawl-reaction friend in trouble",
            );
            self.base.seek_position = view.position;
            self.base.go_near(
                view.position,
                parameters_ai::AI_HIT_DISTANCE,
                crate::ai::GotoFlags::RUN,
                ctx,
            );
            self.base.launch_timer(1, ctx.frame);
        }
        false
    }

    // Brawl approach: refresh chase on timer; on reach,
    // attempt the hit.

    fn wondering_brawl_approaching(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // If target moved > 3 units from the seek position,
                // update seek position and re-issue the approach.
                // Otherwise re-arm the timer.
                let view = ctx.expect_entity_view(
                    self.base.friend_in_trouble,
                    "brawl-approach friend in trouble",
                );
                let dx = (self.base.seek_position.x - view.position.x).abs();
                let dy = (self.base.seek_position.y - view.position.y).abs();
                if dx.max(dy) > 3.0 {
                    self.base.seek_position = view.position;
                    self.base.go_near(
                        view.position,
                        parameters_ai::AI_HIT_DISTANCE,
                        crate::ai::GotoFlags::RUN,
                        ctx,
                    );
                }
                self.base.launch_timer(1, ctx.frame);
            }
            StimulusType::EventReachPoint => {
                let Some(friend) = self.base.friend_in_trouble else {
                    tracing::error!(
                        actor = self.base.me,
                        "brawl approach reached its target without a friend in trouble"
                    );
                    // Return the actor to duty, as in the shipped game.
                    // Log the bad state without making debug and
                    // production simulation behavior diverge.
                    self.return_to_duty_default(env)?;
                    return Ok(false);
                };
                let friend_view =
                    ctx.expect_entity_view(friend, "brawl-approach friend in trouble");
                if friend_view.ai_state == AiState::Sleeping {
                    let fit = friend.get();
                    self.money_fight_enemies.retain(|h| *h != fit);
                    self.base.friend_in_trouble = None;
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlHitting);
                    self.base
                        .outbox
                        .reentrant
                        .self_stimuli
                        .push(StimulusType::EventDone.into());
                } else {
                    let dx = friend_view.position.x - ctx.position.x;
                    let dy = friend_view.position.y - ctx.position.y;
                    if dx.hypot(dy) > parameters_ai::AI_HIT_DISTANCE as f32 + 3.0 {
                        self.base.go_near(
                            friend_view.position,
                            parameters_ai::AI_HIT_DISTANCE,
                            crate::ai::GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        self.base.stop_all();
                        let antagonist = ctx.entity_id(friend).unwrap_or_else(|| {
                            panic!("brawl hit friend handle {friend} has no live typed entity view")
                        });
                        let mut sequence = crate::sequence::Sequence::new();
                        sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                            1,
                            crate::element::Command::HitCmd,
                            self.base.owner_entity_id,
                            Some(antagonist),
                        ));
                        self.base.outbox.actor.launch_sequences.push(sequence);
                        self.set_state(AiState::Wondering, Substate::WonderingBrawlHitting);
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Brawl hit resolution; civilians panic, chase chain continues.

    fn wondering_brawl_hitting(&mut self, stimulus_type: StimulusType) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // The owner-work continuation performs the civilian sweep,
            // synchronously settles the later officer notification, then
            // invokes the remaining brawler tail. Keeping the tail out of
            // this initial outbox is essential because owner work otherwise
            // drains ahead of cross-NPC calls.
            self.base.outbox.reentrant.brawl_hitting_completion_pending = true;
            self.nearby_civilians_panic_180();
        }
        false
    }

    // Brawl-got-hit: pivot to BrawlRecovering, register
    // attacker as new money-fight enemy, set thunderstorm
    // emoticon. If the NPC is lying, queue StandUp; otherwise
    // self-fire EventDone so BrawlRecovering immediately picks
    // the next victim.

    fn wondering_brawl_got_hit(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(AiState::Wondering, Substate::WonderingBrawlRecovering);
            // maybe_officer_sees_me_fighting().
            self.maybe_officer_sees_me_fighting(ctx, tick);
            // Set the thunderstorm emoticon.
            self.base.set_emoticon(EmoticonType::Thunderstorm);
            // Insert friend_in_trouble into money_fight_enemies
            // (asserts soldier + non-self).
            if let Some(fit) = self.base.friend_in_trouble
                && fit.get() != self.base.me
            {
                let is_soldier = ctx
                    .expect_entity_view(fit, "brawl-got-hit attacker")
                    .is_soldier();
                if is_soldier && !self.money_fight_enemies.contains(&fit.get()) {
                    self.money_fight_enemies.push(fit.get());
                }
            }
            // If lying, launch StandUp; else recurse
            // Think(EventDone) into BrawlRecovering.
            if ctx.posture == crate::element::Posture::Lying {
                self.base.stop_all();
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::StandUp);
            } else {
                // Self-fire EventDone so the new
                // BrawlRecovering substate picks up the next
                // victim immediately on this same tick.
                self.base.fire_self_stimulus(StimulusType::EventDone);
            }
        }
        false
    }

    // Brawl recovery: go punch the next enemy, or stop brawling.

    fn wondering_brawl_recovering(
        &mut self,
        stimulus_type: StimulusType,
        _ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            return Err(DutyCall {
                tail: crate::ai::DutyTail::MoneyFight {
                    operation: crate::ai::MoneyFightOperation::RecoverBrawl,
                },
                ..DutyCall::new(DutyFlags::empty(), false)
            });
        }
        Ok(false)
    }

    // Reached looting body.  Either re-transition to loot
    // (distant), flag a tied body, or kick off the SEARCH
    // sequence.

    fn wondering_approaching_to_loot(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            let body = self.required(
                self.base.detected_body,
                "a detected body",
                "reacting to a body",
            );
            let v = ctx.expect_entity_view(body, "loot-approach body");
            let body_pos = v.position;
            let is_tied = v.posture == crate::element::Posture::Tied;
            let dx = body_pos.x - ctx.position.x;
            let dy = body_pos.y - ctx.position.y;
            let dist = dx.abs().max(dy.abs());
            if dist > 100.0 {
                // Too far — let Looting handle re-entry.
                // Kick the state machine via a 1-tick timer;
                // the Looting arm handles the follow-up.  We
                // can't re-enter `think()` from inside an arm,
                // so fall back to a short timer that reaches
                // the same code path.
                self.set_state_with_timer(AiState::Wondering, Substate::WonderingLooting, 1, ctx);
            } else if is_tied {
                // Spot the tied body and transition to
                // body-seek; emit the reconnaissance report
                // update.
                self.base.my_reconnaissance_report.add_seen_body(body.get());
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Body, body_pos);
                // Re-issue Think(EventReachPoint) via a 1-tick
                // timer (see comment above).
                self.set_state_with_timer(AiState::Seeking, Substate::SeekingBody, 1, ctx);
            } else {
                // Start SEARCH sequence, transition to Looting.
                use crate::element::Command;
                use crate::sequence::{Sequence, SequenceElement};
                self.old_money = ctx
                    .entity_view(self.base.me)
                    .map(|v| v.current_money.min(u16::MAX as u32) as u16)
                    .unwrap_or_else(|| {
                        panic!(
                            "looting soldier {} is missing its required owner entity view",
                            self.base.me
                        )
                    });
                self.set_state(AiState::Wondering, Substate::WonderingLooting);
                self.base.stop_all();
                let owner = self.base.owner_entity_id;
                let antagonist = Some(crate::element::EntityId::Soldier(
                    crate::entity_id::SoldierId(body.get()),
                ));
                let mut seq = Sequence::new();
                seq.append_element(SequenceElement::new_interaction(
                    1,
                    Command::SearchCmd,
                    owner,
                    antagonist,
                ));
                self.base.outbox.actor.launch_sequences.push(seq);
            }
        }
        false
    }

    // Looting: inspect gain, move to next victim or return to duty.

    fn wondering_looting(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventDone {
            let current_money = ctx
                .entity_view(self.base.me)
                .map(|v| v.current_money.min(u16::MAX as u32) as u16)
                .unwrap_or_else(|| {
                    panic!(
                        "looting soldier {} is missing its required owner entity view",
                        self.base.me
                    )
                });
            if current_money > self.old_money {
                self.base
                    .set_transient_emoticon(EmoticonType::Sun, 20, ctx.frame);
                self.base.say(Remark::SearchingSoldierGold);
            } else {
                self.base
                    .set_transient_emoticon(EmoticonType::Cloud, 20, ctx.frame);
                self.base.say(Remark::SearchingSoldierNothing);
            }

            while self.money_fight_victims.first().is_some_and(|h| {
                ctx.expect_entity_view(*h as HumanHandle, "money-fight victim")
                    .looted_after_money_fight
            }) {
                self.money_fight_victims.remove(0);
            }
            if !self.money_fight_victims.is_empty() {
                let next = self.money_fight_victims.remove(0);
                self.base.detected_body = Some(AiEntityHandle::new(next));
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::SetLootedAfterMoneyFight {
                        target: next,
                        looted: true,
                    },
                );
                self.set_state(AiState::Wondering, Substate::WonderingApproachingToLoot);
                let view = ctx.expect_entity_view(next as HumanHandle, "money-fight victim");
                self.base.go_near(
                    view.position,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    crate::ai::GotoFlags::empty(),
                    ctx,
                );
            } else {
                return Err(crate::ai::DutyCall::new(DutyFlags::KEEP_EMOTICON, false));
            }
        }
        Ok(false)
    }

    // Beer went away: try next remembered beer, else return to duty.

    fn wondering_ale_away(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            if !self.other_seen_ale.is_empty() {
                // Remember next beer as object of desire.
                let next = self.other_seen_ale.remove(0);
                let next = AiEntityHandle::new(next);
                self.base.object_of_desire = Some(next);
                self.base.interesting_object = Some(next);
                // Enter the wondering / approaching-ale state.
                self.set_state(AiState::Wondering, Substate::WonderingApproachingAle);
                // Approach an accessible point within AI_STOP_BEFORE_MONEY_DISTANCE of the object.
                if let Some(pos) = ctx.entity_position(next) {
                    self.base.go_near(
                        pos,
                        parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                        crate::ai::GotoFlags::FIND_ACCESSIBLE,
                        ctx,
                    );
                }
                // Remember patrol return point
                self.return_to_patrol_point = ctx.position;
                // Quick recheck
                self.base.launch_timer(1, ctx.frame);
            } else {
                self.return_to_duty_default(env)?;
            }
        }
        Ok(false)
    }

    // Officer sees brawl: close distance, clear emoticon.

    fn wondering_officer_seeing_brawl(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(
                AiState::Wondering,
                Substate::WonderingOfficerApproachingBrawl,
            );
            self.base.set_emoticon(EmoticonType::None);
            // Approach to within 100 units of the friend in trouble.
            let view = ctx.expect_entity_view(
                self.base.friend_in_trouble,
                "officer-seeing-brawl friend in trouble",
            );
            self.base
                .go_near(view.position, 100, crate::ai::GotoFlags::empty(), ctx);
        }
        false
    }

    // Officer reached the brawl; enter finish-brawl state,
    // set thunderstorm mood.

    fn wondering_officer_approaching_brawl(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> AiFlow<bool> {
        if stimulus_type == StimulusType::EventReachPoint
            && self.base.current_remark != Remark::TheSoundOfSilence
        {
            self.base.launch_timer(50, ctx.frame);
        } else if matches!(
            stimulus_type,
            StimulusType::EventReachPoint | StimulusType::EventTimer
        ) {
            return Err(DutyCall {
                tail: crate::ai::DutyTail::MoneyFight {
                    operation: crate::ai::MoneyFightOperation::FinishBrawl,
                },
                ..DutyCall::new(DutyFlags::empty(), false)
            });
        }
        Ok(false)
    }

    // Finishing-brawl orchestration: chain CallYourTalk1..3,
    // then timer dismisses soldiers and waits on the antagonist.

    fn wondering_officer_finishing_brawl(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer | StimulusType::EventMyTalk2 => {
                // forget_all_nearby_coins().
                self.forget_all_nearby_coins(ctx);
                // Walk list_us, send a return-to-duty request to each soldier
                // that isn't the antagonist.
                let antagonist = self.base.antagonist;
                let us: Vec<HumanHandle> = self
                    .base
                    .list_us
                    .iter()
                    .copied()
                    .filter(|h| Some(AiEntityHandle::new(*h)) != antagonist && *h != self.base.me)
                    .collect();
                for target in us {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target,
                            stimulus_type: StimulusType::EventReturnToDuty,
                            info: crate::ai::StimulusInfo::None,
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                }
                self.base.list_us.clear();

                // CallCleanUpAfterBrawl to antagonist.
                if let Some(antagonist) = antagonist {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallCleanUpAfterBrawl,
                            info: crate::ai::StimulusInfo::None,
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                    self.set_state_with_timer(
                        AiState::Wondering,
                        Substate::WonderingOfficerFinishingBrawlWaiting,
                        10,
                        ctx,
                    );
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            StimulusType::CallYourTalk3 => {
                self.base.say(Remark::OfficerEndsConversation);
            }
            _ => {}
        }
        Ok(false)
    }

    // Keep waiting while antagonist still
    // approaching/awakening a victim; else end.

    fn wondering_officer_finishing_brawl_waiting(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            // If antagonist is still approaching or awakening
            // the brawl victim, re-arm timer; else end.
            let still_waiting = matches!(
                ctx.expect_entity_view(self.base.antagonist, "officer-finishing-brawl antagonist",)
                    .ai_substate,
                Substate::WonderingApproachingBrawlVictim | Substate::WonderingAwakenBrawlVictim
            );
            if still_waiting {
                self.base.launch_timer(10, ctx.frame);
            } else {
                self.return_to_duty_default(env)?;
            }
        }
        Ok(false)
    }

    // Soldier side of the "officer finished brawl" lecture:
    // 3-variant excuse speeches until the timer fires.

    fn wondering_soldier_looking_officer_who_finished_brawl(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // forget_all_nearby_coins(); return_to_duty(sim, );
                self.forget_all_nearby_coins(ctx);
                self.return_to_duty_default(env)?;
            }
            StimulusType::EventMyTalk1
            | StimulusType::EventMyTalk2
            | StimulusType::EventMyTalk3 => {
                self.base.set_emoticon(EmoticonType::None);
                // antagonist.think(CallYourTalk1).
                // Note: always forward as CallYourTalk1
                // regardless of which MyTalk variant
                // triggered — the 3 cycle variants just vary
                // which BadExcuse sample plays; the callback
                // is always CallYourTalk1 on the officer.
                if let Some(antagonist) = self.base.antagonist {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallYourTalk1,
                            info: crate::ai::StimulusInfo::None,
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Watcher finishes looking at tower guard; back to duty.

    fn wondering_approaching_brawl_victim(&mut self, stimulus_type: StimulusType) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            use crate::element::Command;
            use crate::sequence::{Sequence, SequenceElement};
            self.set_state(AiState::Wondering, Substate::WonderingAwakenBrawlVictim);
            self.base.stop_all();
            let owner = self.base.owner_entity_id;
            if let Some(body) = self.base.detected_body {
                let antagonist = Some(crate::element::EntityId::Soldier(
                    crate::entity_id::SoldierId(body.get()),
                ));
                let mut seq = Sequence::new();
                seq.append_element(SequenceElement::new_interaction(
                    1,
                    Command::WakeUp,
                    owner,
                    antagonist,
                ));
                self.base.outbox.actor.launch_sequences.push(seq);
            }
        }
        false
    }

    // Done awakening the victim: move to the next fight victim.

    fn wondering_awaken_brawl_victim(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.awake_next_money_fight_victim_if_any(env)?;
        }
        Ok(false)
    }

    fn think_expected_seeking_event(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        Ok(match self.base.current_substate {
            Substate::SeekingSeekpoint => self.seeking_seekpoint(env, stimulus_type, global)?,

            Substate::SeekingSeekpointWatching => {
                self.seeking_seekpoint_watching(sim, stimulus_type)
            }

            Substate::SeekingSeekpointWatchingSidewards => {
                self.seeking_seekpoint_watching_sidewards(env, stimulus_type, global)?
            }

            Substate::SeekingSeekpointPassedAmbushPointLeft => {
                self.seeking_seekpoint_passed_ambush_point_left(env, stimulus_type, global)?
            }

            Substate::SeekingSeekpointPassedAmbushPointRight => {
                self.seeking_seekpoint_passed_ambush_point_right(env, stimulus_type, global)?
            }

            Substate::SeekingSeekpointCheckingAmbushPoint => {
                self.seeking_seekpoint_checking_ambush_point(stimulus_type, global, ctx)
            }

            Substate::SeekingSeekpointApproachingBeggar => {
                self.seeking_seekpoint_approaching_beggar(env, stimulus_type, global)?
            }

            Substate::SeekingSeekpointIdentifyingBeggar1 => {
                self.seeking_seekpoint_identifying_beggar1(stimulus_type, ctx)
            }

            Substate::SeekingSeekpointIdentifyingBeggar2 => {
                self.seeking_seekpoint_identifying_beggar2(env, stimulus_type, global)?
            }

            Substate::SeekingHeardstepsPreReactiontime => {
                self.seeking_heardsteps_pre_reactiontime(stimulus_type, ctx)
            }

            Substate::SeekingHeardstepsReactiontime => {
                self.seeking_heardsteps_reactiontime(stimulus_type, ctx, tick)
            }

            Substate::SeekingHeardsteps => self.seeking_heardsteps(env, stimulus_type, global)?,

            Substate::SeekingJustWatching => self.seeking_just_watching(sim, stimulus_type, ctx),

            Substate::SeekingJustWatchingSidewards => {
                self.seeking_just_watching_sidewards(env, stimulus_type)?
            }

            Substate::SeekingBodyReactiontime => false,

            Substate::SeekingArrowReactiontime => {
                self.seeking_arrow_reactiontime(stimulus_type, ctx)
            }

            Substate::SeekingArrow => self.seeking_arrow(env, stimulus_type, global)?,

            Substate::SeekingArrowJustWatching | Substate::SeekingArrowJustWatchingSidewards => {
                self.seeking_arrow_just_watching(stimulus_type, env)?
            }

            Substate::SeekingCombatAlertReactiontime => {
                self.seeking_combat_alert_reactiontime(stimulus_type, ctx)
            }

            Substate::SeekingCombatAlert => {
                self.seeking_combat_alert(env, stimulus_type, global)?
            }

            Substate::SeekingGotStopEvent => self.seeking_got_stop_event(stimulus_type, ctx),

            Substate::SeekingWaitForAlertingCivilian => {
                self.seeking_wait_for_alerting_civilian(env, stimulus, stimulus_type)?
            }

            Substate::SeekingGetReportFromCivilian => {
                self.seeking_get_report_from_civilian(env, stimulus_type)?
            }

            Substate::SeekingGetAlertingReportFromCivilian => {
                self.seeking_get_alerting_report_from_civilian(stimulus_type, ctx)
            }

            Substate::SeekingGetAlertingReportFromCivilianLook => {
                self.seeking_get_alerting_report_from_civilian_look(stimulus_type, global, env)?
            }

            Substate::SeekingOfficerCallSoldier => self.seeking_officer_call_soldier(stimulus_type),

            Substate::SeekingOfficerWaitForSoldier => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerInstructSoldier => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerWaitForInstructedSoldier => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerGetReportFromSoldier => {
                unreachable!("report conversation must execute through the engine")
            }

            Substate::SeekingSoldierCalledByOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingSoldierGoToOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingSoldierGetInstructedByOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingSoldierReturnToOfficer => {
                unreachable!("report conversation must execute through the engine")
            }

            Substate::SeekingSoldierGiveReportToOfficer => {
                unreachable!("report conversation must execute through the engine")
            }

            Substate::SeekingOfficerCallGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerWaitForGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerInstructGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerInstructGroupPointing => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerWaitForInstructedGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerWaitInsideHouseToInstructGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingOfficerLeavingHouseToInstructGroup => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingGroupCalledByOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingGroupGoToOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingGroupGetInstructedByOfficer => {
                unreachable!("officer rendezvous must execute through the engine")
            }

            Substate::SeekingRunningToOfficer => {
                self.seeking_running_to_officer(env, stimulus_type)?
            }

            Substate::SeekingRunningToOfficerSeen => {
                unreachable!("alert report conversation must execute through the engine")
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerStart => {
                unreachable!("alert report conversation must execute through the engine")
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerPoint => {
                unreachable!("alert report conversation must execute through the engine")
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd => {
                unreachable!("alert report conversation must execute through the engine")
            }

            Substate::SeekingOfficerWaitForAlertingSoldier => {
                self.seeking_officer_wait_for_alerting_soldier(env, stimulus, stimulus_type)?
            }

            Substate::SeekingOfficerGetAlertingReportFromSoldier => {
                self.seeking_officer_get_alerting_report_from_soldier(stimulus_type, env)?
            }

            Substate::SeekingKnightWatchingTowerGuard => {
                self.seeking_knight_watching_tower_guard(env, stimulus_type, global)?
            }

            Substate::SeekingNet => self.seeking_net(env, stimulus_type, global)?,

            Substate::SeekingOfficerLookingForSoldiers1
            | Substate::SeekingOfficerLookingForSoldiers2
            | Substate::SeekingOfficerLookingForSoldiers3 => {
                self.seeking_officer_looking_for_soldiers1(sim, stimulus_type)
            }

            Substate::SeekingOfficerLookingForSoldiers1Sidewards
            | Substate::SeekingOfficerLookingForSoldiers2Sidewards => {
                self.seeking_officer_looking_for_soldiers1_sidewards(stimulus_type, ctx)
            }

            Substate::SeekingOfficerLookingForSoldiers3Sidewards => {
                self.seeking_officer_looking_for_soldiers3_sidewards(env, stimulus_type)?
            }

            Substate::SeekingCharly => self.seeking_charly(env, stimulus_type)?,

            Substate::SeekingCharlyWatching => {
                self.seeking_charly_watching(stimulus_type, global, env)?
            }

            Substate::SeekingDetectedCharly => self.seeking_detected_charly(env, stimulus_type)?,

            Substate::SeekingSendCharlyToOfficer => {
                self.seeking_send_charly_to_officer(env, stimulus_type)?
            }

            Substate::SeekingLookingResurrectedCharly => {
                self.seeking_looking_resurrected_charly(env, stimulus_type)?
            }

            Substate::SeekingCharlySentToOfficer => {
                self.seeking_charly_sent_to_officer(stimulus_type, ctx)
            }

            Substate::SeekingCharlyGoToOfficer => {
                self.seeking_charly_go_to_officer(env, stimulus_type)?
            }

            Substate::SeekingCharlyGoToOfficerSeen => {
                self.seeking_charly_go_to_officer_seen(env, stimulus_type)?
            }

            Substate::SeekingCharlyGetLectureByOfficer => {
                self.seeking_charly_get_lecture_by_officer(stimulus_type)
            }

            Substate::SeekingCharlyGetLectureByOfficer2 => {
                self.seeking_charly_get_lecture_by_officer2(env, stimulus_type)?
            }

            Substate::SeekingOfficerWaitForCharly => {
                self.seeking_officer_wait_for_charly(env, stimulus, stimulus_type)?
            }

            Substate::SeekingOfficerLectureCharly => {
                self.seeking_officer_lecture_charly(stimulus_type, ctx)
            }

            Substate::SeekingOfficerLectureCharlyPointing => {
                self.seeking_officer_lecture_charly_pointing(env, stimulus_type)?
            }

            Substate::SeekingCivilianRunningToSoldierSeen => {
                self.seeking_civilian_running_to_soldier_seen(env, stimulus_type)?
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierStart => {
                self.seeking_civilian_give_alerting_report_to_soldier_start(stimulus_type, ctx)
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint => {
                self.seeking_civilian_give_alerting_report_to_soldier_point(stimulus_type, ctx)
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd => {
                self.seeking_civilian_give_alerting_report_to_soldier_end(stimulus_type)
            }

            // Reserve overview re-evaluates via battle planning.
            _ => false,
        })
    }

    fn seeking_seekpoint(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventReachPoint && self.actual_seek_point.is_some() {
            self.reached_seek_point(env, global)?;
        }
        Ok(false)
    }

    fn seeking_seekpoint_watching(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Random LR/RL.
            self.set_state(
                AiState::Seeking,
                Substate::SeekingSeekpointWatchingSidewards,
            );
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2) != 0 {
                    LookDirection::LeftRight
                } else {
                    LookDirection::RightLeft
                },
            );
        }
        false
    }

    fn seeking_seekpoint_watching_sidewards(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventDone || stimulus_type == StimulusType::EventTimer {
            // Check if more directions to look
            if let Some(&dir) = self.seek_point_view_directions.first() {
                self.seek_point_view_directions.remove(0);
                self.base.face_direction(dir, ctx);
                self.base.number_of_looks = 0;
                self.set_state_with_timer(
                    AiState::Seeking,
                    Substate::SeekingSeekpointWatching,
                    parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32,
                    ctx,
                );
            } else {
                // No directions left — move to next seek point
                self.seek_next_point(env, global)?;
            }
        }
        Ok(false)
    }

    fn seeking_seekpoint_passed_ambush_point_left(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                // The original game dispatches the reach-point event re-entrantly
                // here; do the same work inline instead of synthesizing
                // a one-frame timer.
                if self.actual_seek_point.is_some() {
                    self.reached_seek_point(env, global)?;
                }
            }
            StimulusType::EventTimer => {
                self.base.stop_all();
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                // Look LEFT.
                self.base.outbox.actor.look_sidewards = Some(LookDirection::Left);
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_seekpoint_passed_ambush_point_right(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                if self.actual_seek_point.is_some() {
                    self.reached_seek_point(env, global)?;
                }
            }
            StimulusType::EventTimer => {
                self.base.stop_all();
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                // Look RIGHT.
                self.base.outbox.actor.look_sidewards = Some(LookDirection::Right);
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_seekpoint_checking_ambush_point(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // Resume walking to seek point
            let goto_flags = if self.seek_flags.contains(SeekFlags::WALKING) {
                GotoFlags::empty()
            } else {
                GotoFlags::RUN
            };
            let seek_point_id = self
                .actual_seek_point
                .expect("ambush-point check lost its actual seek point");
            let seek_position = resolve_seek_point_id(
                seek_point_id,
                &self.personal_seek_point_1,
                &self.personal_seek_point_2,
                global,
            )
            .unwrap_or_else(|| panic!("actual seek point {seek_point_id:?} no longer resolves"))
            .position;
            self.go_to(
                AiState::Seeking,
                Substate::SeekingSeekpoint,
                seek_position,
                goto_flags,
                ctx,
            );
        }
        false
    }

    // -- Beggar identification substates --

    fn seeking_seekpoint_approaching_beggar(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        // Soldier is walking toward the beggar's last known
        // position (set by seek_next_point → go_near).
        // On arrival, stop and begin identification.
        if stimulus_type == StimulusType::EventReachPoint {
            let beggar = self.required(
                self.beggar_to_examine,
                "a beggar-to-examine",
                "approaching a beggar seek point",
            );
            // The arrival is only an
            // identification when the beggar's max-norm distance is below 100.
            // The go_near(50) request only bounds the *path goal*; the beggar
            // can have walked away, or the point can be reached from the far
            // side of an obstacle, so the arrival must be re-measured against
            // the beggar's live body position. Otherwise the soldier menaces
            // thin air instead of resuming the search at the next seek point.
            let beggar_view = ctx.entity_view(beggar).unwrap_or_else(|| {
                panic!(
                    "beggar {} disappeared before the approach distance check",
                    beggar
                )
            });
            let beggar_world = beggar_view.detection_position_world;
            if ai_max_norm_distance_world(&beggar_world, &ctx.self_body_position_world) >= 100.0 {
                // Too far to control this beggar — carry on searching.
                self.seek_next_point(env, global)?;
                return Ok(false);
            }
            self.base.stop_all();
            self.set_state(
                AiState::Seeking,
                Substate::SeekingSeekpointIdentifyingBeggar1,
            );
            self.base.say(Remark::ControlsBeggar);

            // Original authors one two-level sequence, not two
            // independent sequence-element launches: fast turning must
            // retain actor ownership until it terminates before the
            // menace/equip command can begin.
            use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};
            let beggar_position = ctx
                .entity_view(beggar)
                .unwrap_or_else(|| {
                    panic!("beggar {} disappeared before identification turn", beggar)
                })
                .position;
            let turn_direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                beggar_position.x - ctx.position.x,
                beggar_position.y - ctx.position.y,
            );
            let owner = self.base.owner_entity_id;
            let mut sequence = Sequence::new();
            let mut turn =
                SequenceElement::new_generic(1, crate::element::Command::TurnFast, owner);
            turn.set_property(Field::Direction, FieldValue::Integer(turn_direction as u32));
            sequence.append_element(turn);

            // Archers equip bow; melee soldiers menace. Timer = 50
            // (NPC target) / 100 (PC target) for archers, 30 for
            // melee.
            if self.is_archer() {
                sequence.append_element(SequenceElement::new(
                    2,
                    crate::element::Command::EquipBow,
                    owner,
                ));
                // The original game tests whether the examined beggar is an NPC here instead
                // of retaining a discriminator on the soldier AI.  Resolve
                // the live target as well: this substate can be restored
                // directly from a save, in which case our compatibility
                // cache has never been populated.
                let beggar_is_npc = ctx
                    .entity_view(beggar)
                    .unwrap_or_else(|| {
                        panic!(
                            "beggar {} disappeared before identification timer setup",
                            beggar
                        )
                    })
                    .is_civilian();
                let timer = if beggar_is_npc { 50 } else { 100 };
                self.base.launch_timer(timer, ctx.frame);
            } else {
                sequence.append_element(SequenceElement::new(
                    2,
                    crate::element::Command::StartMenace,
                    owner,
                ));
                self.base.launch_timer(30, ctx.frame);
            }
            self.base.outbox.actor.launch_sequences.push(sequence);
        }
        Ok(false)
    }

    fn seeking_seekpoint_identifying_beggar1(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        // First inspection phase: timer fires after the
        // menace/equip-bow animation completes.
        if stimulus_type == StimulusType::EventTimer {
            let beggar = self.required(
                self.beggar_to_examine,
                "a beggar-to-examine",
                "identifying a beggar",
            );
            // This logic queries
            // the examined beggar's NPC status at the instant this timer fires.
            // Do not use `beggar_is_npc`: it is only a compatibility cache
            // populated while choosing the next seek point, and therefore is
            // false when a save resumes in this identification substate.
            let beggar_is_npc = ctx
                .entity_view(beggar)
                .unwrap_or_else(|| panic!("beggar {} disappeared during identification", beggar))
                .is_civilian();
            if beggar_is_npc {
                // Real beggar: NPC shows face and identifies
                // themselves. Transition to phase 2 (wait,
                // then resume seeking).
                // Launch a `BeggarShowFace` sequence element on
                // the beggar via `pending_launch_on_target`,
                // which carries (target, cmd) to the
                // engine-side sequence-manager drain.
                self.base
                    .outbox
                    .actor
                    .launch_on_target
                    .push((beggar, crate::element::Command::BeggarShowFace));
                // Original immediately follows the show-face launch
                // with the beggar's identification remark.
                // Keep both calls in the ordered actor-effect prefix:
                // The state change below snapshots that prefix before its
                // synchronous script callback.
                self.base
                    .outbox
                    .actor
                    .say_on_target
                    .push((beggar, crate::ai::Remark::CivBeggarIdentifiesHimself));
                self.set_state_with_timer(
                    AiState::Seeking,
                    Substate::SeekingSeekpointIdentifyingBeggar2,
                    50,
                    ctx,
                );
            } else {
                // Disguised PC detected! Set as primary target
                // and begin combat.
                self.base.primary_target = Some(beggar);
                self.list_them.clear();
                self.list_them.push(beggar.get());

                if self.is_archer() {
                    // False beggar stands up via `LeaveBeggar`,
                    // then the archer transitions to
                    // AttackingBowShooting and shoots.
                    self.base
                        .outbox
                        .actor
                        .launch_on_target
                        .push((beggar, crate::element::Command::LeaveBeggar));
                    self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                    self.shoot_arrow_at(beggar.get(), ctx);
                } else {
                    // Melee: call PC for duel.
                    self.begin_swordfight(ctx);
                }
            }
        }
        false
    }

    fn seeking_seekpoint_identifying_beggar2(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        // Second phase (NPC path only): the real beggar has
        // identified themselves. Timer fires → resume seeking.
        if stimulus_type == StimulusType::EventTimer {
            self.seek_next_point(env, global)?;
        }
        Ok(false)
    }

    // Pre-reactiontime gates whether to investigate himself
    // or just watch:
    //   - SOLDIER/KNIGHT: decide whether to follow footsteps
    //   - OFFICER: only if no patrol *and* close enough to noise.
    // If "do not investigate yourself" → JustWatching, else
    // HeardstepsReactiontime.  Both arms set Q-mark + face + 60-tick
    // timer.

    fn seeking_heardsteps_pre_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            let do_not_investigate = if self.investigating_distraction {
                false
            } else {
                match self.get_rank() {
                    ProfileRank::Officer => {
                        // The original game checks the officer's current patrol list,
                        // patrol, not whether any camp snapshot still names this
                        // officer as chief. A separated member lives in
                        // missed-patrol-member list and must not stop the officer
                        // from investigating a nearby noise himself.
                        let has_patrol = !self.base.patrol.is_empty();
                        let dx = (ctx.position.x - self.base.seek_position.x).abs();
                        let dy = (ctx.position.y - self.base.seek_position.y).abs();
                        const OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE: f32 = 100.0;
                        has_patrol || dx.max(dy) > OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE
                    }
                    // Soldier / knight: defer to ShallIFollowSteps.
                    ProfileRank::Soldier | ProfileRank::Knight => {
                        !self.answer_question(Question::ShallIFollowSteps, ctx)
                    }
                    // The shipped Linux v48 game's stable result for rank-less
                    // soldiers is false here (observed for the
                    // same inactive rank-less soldier in Savegame_023 replays 004
                    // and 005), so preserve that behavior instead of
                    // incorrectly folding RANK_NONE into the soldier question.
                    ProfileRank::None => false,
                }
            };
            self.base.set_emoticon(EmoticonType::QuestionMark);
            if do_not_investigate {
                self.set_state(AiState::Seeking, Substate::SeekingJustWatching);
                self.base
                    .face_position_3d_with_ctx(self.base.seek_position, ctx);
            } else {
                self.base
                    .face_position_3d_with_ctx(self.base.seek_position, ctx);
                self.set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
            }
            self.base
                .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
        }
        false
    }

    fn seeking_heardsteps_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // A plain soldier who can see an officer already
            // heading for the same noise defers to him instead of
            // investigating himself.
            let officer = if self.get_rank() == ProfileRank::Soldier {
                self.near_officer_who_is_wondering_about_the_same_noise(ctx, tick)
            } else {
                None
            };
            if officer.is_some() {
                self.set_state_with_timer(
                    AiState::Default,
                    Substate::DefaultLookingOfficerForAdvice,
                    100,
                    ctx,
                );
            } else {
                let goto_flags = if self.investigating_distraction {
                    GotoFlags::RUN
                } else {
                    GotoFlags::empty()
                };
                self.go_to(
                    AiState::Seeking,
                    Substate::SeekingHeardsteps,
                    self.base.seek_position,
                    goto_flags,
                    ctx,
                );
                self.base.launch_timer(200, ctx.frame);
            }
        }
        false
    }

    fn seeking_heardsteps(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventReachPoint | StimulusType::EventTimer => {
                // Search exactly at the noise source. Original uses
                // the actor's live position, not the remembered noise
                // position, and creates one personal seek point with
                // random look directions before walking to it.
                self.seek_area(
                    env,
                    ctx.position,
                    0,
                    SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                    UNDEFINED_DIRECTION,
                    global,
                )?;
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_just_watching(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Seeking, Substate::SeekingJustWatchingSidewards);
            // Randomly pick a two-step head-turn direction.
            // The engine consumes `pending_look_sidewards`
            // into a sequence of LookLeft / LookRight commands
            // at post-think time.
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2) != 0 {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                },
            );
            self.base
                .launch_timer(parameters_ai::AI_LOOK_TIME as u32, ctx.frame);
        }
        false
    }

    fn seeking_just_watching_sidewards(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, tick, .. } = env;
        if stimulus_type == StimulusType::EventDone {
            // Original has explicit SOLDIER and OFFICER arms and no
            // default arm. A knight can nevertheless inherit this
            // substate from a legacy save, in which case EVENT_DONE
            // deliberately leaves it unchanged.
            match self.get_rank() {
                ProfileRank::Soldier => {
                    self.return_to_duty_default(env)?;
                }
                ProfileRank::Officer => {
                    self.officer_look_for_soldier(ReportType::Noise)?;
                }
                ProfileRank::Knight | ProfileRank::None => {}
            }
        }
        Ok(false)
    }

    // Arrow reactiontime: Say(Arrow), transition to
    // SeekingArrow, run to noise, broadcast a look-there alert,
    // launch 200-tick timer.

    fn seeking_arrow_reactiontime(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.base.say(Remark::Arrow);
            // The original game's arrow-reaction state uses plain
            // Run toward the seek position. The nearby seek-point
            // adjustment already happened when the arrow stimulus arrived;
            // this movement must not add approach movement's stop-distance tolerance.
            self.go_to(
                AiState::Seeking,
                Substate::SeekingArrow,
                self.base.seek_position,
                GotoFlags::RUN,
                ctx,
            );
            let seek_pos = self.base.seek_position;
            if !self.hey_folks_look_there(
                &seek_pos,
                100,
                LookThereContinuation::SeekingArrowReactiontime,
                ctx,
            ) {
                self.base.launch_timer(200, ctx.frame);
            }
        }
        false
    }

    // At noise origin: search the area around the current position.
    // SOLDIER also sets LOOK_FOR_HELP_AFTER_SEEKING; OFFICER
    // does not.

    fn seeking_arrow(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventReachPoint
            || stimulus_type == StimulusType::EventTimer
        {
            let mut flags = SeekFlags::LOCATION_FIRST | SeekFlags::WALKING;
            if self.get_rank() == ProfileRank::Soldier {
                flags |= SeekFlags::LOOK_FOR_HELP_AFTER;
            }
            let here = ctx.position;
            self.seek_area(env, here, 0, flags, UNDEFINED_DIRECTION, global)?;
        }
        Ok(false)
    }

    // Arrow just-watching:
    // EVENT_TIMER → Say(Arrow, MYTALK_1) (officer-only
    // soliloquy; the Say wrapper triggers MyTalk1 callback).
    // EVENT_MYTALK_1 → AlertSoldiers (officers only); if no
    // soldier reachable, return to duty.

    fn seeking_arrow_just_watching(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        match stimulus_type {
            StimulusType::EventTimer => {
                self.base
                    .say_with_flags(Remark::Arrow, SpeechFlags::MYTALK_1);
            }
            StimulusType::EventMyTalk1 => {
                // Asserts officer; in non-officer cases the
                // MYTALK won't fire, so this branch is
                // officer-only.
                let center = self.base.seek_position;
                let flags = (SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER).bits();
                if !self.alert_soldiers(
                    center,
                    flags,
                    env,
                    AlertSoldiersFailureContinuation::ReturnToDuty,
                )? {
                    self.return_to_duty_default(env)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_combat_alert_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.go_near(
                AiState::Seeking,
                Substate::SeekingCombatAlert,
                self.base.seek_position,
                parameters_ai::AI_HELP_FRIEND_IN_TROUBLE_DISTANCE,
                GotoFlags::RUN,
                ctx,
            );
            self.base.launch_timer(10, ctx.frame);
        }
        false
    }

    fn seeking_combat_alert(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventReachPoint {
            // Original does not reevaluate combat here. The officer's
            // hint target becomes the center of a plain lost-enemy
            // search as soon as the soldier reaches it.
            self.seek_area(
                env,
                self.base.seek_position,
                parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::empty(),
                UNDEFINED_DIRECTION,
                global,
            )?;
        }
        Ok(false)
    }

    fn seeking_got_stop_event(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Original adopts the authored alert path before leaving the
            // stopped-seeking state. This explicit gate is required because
            // the generic state-change alert-path switch only covers departures
            // from STATE_DEFAULT, while this transition starts in SEEKING.
            if let Some(alert_path_id) = self.base.alert_path_id
                && !self.changed_to_alert_path
            {
                self.changed_to_alert_path = true;
                self.base.patrol_path =
                    crate::ai::PatrolPath::new(alert_path_id, &ctx.hiking_paths);
                self.base.has_patrol_path = self.base.patrol_path.is_some();
            }
            self.base.set_emoticon(EmoticonType::QuestionMark);
            self.set_state_with_timer(AiState::Wondering, Substate::WonderingLooking1, 30, ctx);
        }
        false
    }

    // ============ CIVILIAN-ALERTS-SOLDIER ================
    // A civilian has run up to this soldier with a CALL_ALERT;
    // these substates walk the soldier through the "listen
    // to civilian → act on report" flow.

    fn seeking_wait_for_alerting_civilian(
        &mut self,
        env: ThinkEnv<'_>,
        _stimulus: &Stimulus,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // Re-check the civilian is still on the
                // alerting path; face + re-arm timer if so,
                // else give up.
                let civilian_substate =
                    ctx.entity_view(self.base.antagonist).map(|v| v.ai_substate);
                let still_alerting = matches!(
                    civilian_substate,
                    Some(
                        Substate::SeekingCivilianRunningToSoldierSeen
                            | Substate::SeekingCivilianGiveAlertingReportToSoldierStart
                            | Substate::SeekingCivilianGiveAlertingReportToSoldierPoint
                            | Substate::SeekingCivilianGiveAlertingReportToSoldierEnd
                    )
                );
                if still_alerting {
                    self.base.face_entity(self.base.antagonist, ctx);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            StimulusType::CallReport => {
                unreachable!("civilian report handoff must execute through the engine")
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_get_report_from_civilian(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        // Non-alerting civilian report — wait out the
        // talk time and return to duty.
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    fn seeking_get_alerting_report_from_civilian(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        // After the talk timer, turn toward the seek point
        // and enter the LOOK substate for a 30-frame reaction
        // window.
        if stimulus_type == StimulusType::EventTimer {
            let seek_pos = self.base.seek_position;
            self.base.face_position_3d_with_ctx(seek_pos, ctx);
            self.set_state_with_timer(
                AiState::Seeking,
                Substate::SeekingGetAlertingReportFromCivilianLook,
                30,
                ctx,
            );
        }
        false
    }

    fn seeking_get_alerting_report_from_civilian_look(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, tick, .. } = env;
        // Act on the civilian's report based on rank.
        if stimulus_type == StimulusType::EventTimer {
            let seek_pos = self.base.seek_position;
            match self.get_rank() {
                ProfileRank::Officer => {
                    if self.answer_question(Question::ShallISeekBeforeAlertingSoldiers, ctx) {
                        self.seek_area(
                            env,
                            seek_pos,
                            0,
                            SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                            UNDEFINED_DIRECTION,
                            global,
                        )?;
                    } else if !self.alert_soldiers(
                        seek_pos,
                        0,
                        env,
                        AlertSoldiersFailureContinuation::ReturnToDuty,
                    )? {
                        self.return_to_duty_default(env)?;
                    }
                }
                ProfileRank::Soldier => {
                    if self.answer_question(Question::ShallISeekBeforeAlertingOfficer, ctx) {
                        self.seek_area(
                            env,
                            seek_pos,
                            parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                            SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                            UNDEFINED_DIRECTION,
                            global,
                        )?;
                    } else {
                        self.alert_officer(crate::ai::OfficerAlertCaller::SeekHint {
                            center: seek_pos,
                        })?;
                    }
                }
                ProfileRank::Knight => {
                    self.seek_area(
                        env,
                        seek_pos,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                        global,
                    )?;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Resume the statement after the soldier-report branch's synchronous
    /// officer-alert call. A failed route is consumed by officer alerting and
    /// makes the caller seek around the retained civilian report position.

    // ============ OFFICER-SOLDIER COORDINATION ============

    // -------- Officer gives instructions to individual soldier --------

    fn seeking_officer_call_soldier(&mut self, stimulus_type: StimulusType) -> bool {
        // Officer turned to face soldier, now calls them
        if stimulus_type == StimulusType::EventDone {
            let antagonist = self.required(
                self.base.antagonist,
                "an antagonist",
                "calling an individual soldier",
            );
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::RequestThinkResult {
                    target: antagonist.get(),
                    caller: self.base.me,
                    stimulus_type: StimulusType::CallHey,
                    info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    continuation: ThinkResultContinuation::OfficerCalledSoldier,
                });
        }
        false
    }

    // -------- Soldier alerts officer --------

    fn seeking_running_to_officer(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, .. } = env;
        // Soldier running to officer to alert them
        match stimulus_type {
            StimulusType::EventTimer => {
                // Check if officer has moved
                if let Some(antagonist) = ctx.entity_view(self.base.antagonist) {
                    let pos = antagonist.position;
                    let dx = pos.x - self.gather_position.x;
                    let dy = pos.y - self.gather_position.y;
                    let talk_sq = (parameters_ai::AI_TALK_DISTANCE as f32)
                        * (parameters_ai::AI_TALK_DISTANCE as f32);
                    if dx * dx + dy * dy > talk_sq {
                        // Officer moved — update and retry
                        let forecast = antagonist.forecasted_destination.resolve(sim).position;
                        self.gather_position = forecast;
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            forecast,
                            parameters_ai::AI_TALK_DISTANCE,
                            GotoFlags::RUN,
                            ctx,
                        );
                    }
                }
                self.base.launch_timer(50, ctx.frame);
            }
            StimulusType::EventReachPoint => {
                let ant = ctx.entity_view(self.base.antagonist);
                let officer_ok = ant.is_some_and(|a| {
                    a.ai_state == AiState::Default
                        || a.ai_substate == Substate::SeekingOfficerWaitForInstructedSoldier
                        || a.ai_substate == Substate::SeekingDetectedCharly
                        || a.ai_substate == Substate::SeekingOfficerWaitForInstructedGroup
                });
                if officer_ok {
                    let officer_pos = ant.unwrap().position;
                    let dx = officer_pos.x - ctx.position.x;
                    let dy = officer_pos.y - ctx.position.y;
                    let talk_sq = (parameters_ai::AI_TALK_DISTANCE as f32)
                        * (parameters_ai::AI_TALK_DISTANCE as f32);
                    if dx * dx + dy * dy > talk_sq {
                        // Too far — retry
                        let forecast = ant.unwrap().forecasted_destination.resolve(sim).position;
                        self.gather_position = forecast;
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            forecast,
                            parameters_ai::AI_TALK_DISTANCE,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // Close enough — treat as seen
                        // Clear friend detection list — we've reached the officer.
                        self.base
                            .outbox
                            .actor
                            .delete_detectable_type(crate::element::DetectableType::Friend);
                        self.set_state(AiState::Seeking, Substate::SeekingRunningToOfficerSeen);
                        // Original recursively calls
                        // Think(EVENT_REACHPOINT) here.  Keep that
                        // same-frame lifecycle edge in the reentrant
                        // drain; a one-frame timer lets an officer who
                        // is no longer waiting leave this soldier in
                        // the transient Seen state for a frame.
                        self.base
                            .outbox
                            .reentrant
                            .self_stimuli
                            .push(StimulusType::EventReachPoint.into());
                    }
                } else {
                    // Officer busy — look for another
                    self.alert_officer(crate::ai::OfficerAlertCaller::ReturnToDuty)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // -------- Officer is alerted by soldier --------

    fn seeking_officer_wait_for_alerting_soldier(
        &mut self,
        env: ThinkEnv<'_>,
        _stimulus: &Stimulus,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, tick, .. } = env;
        match stimulus_type {
            StimulusType::CallYourTalk0 => {
                self.base.say(Remark::OfficerAsksWhatsup);
            }
            StimulusType::EventTimer => {
                let antagonist = self.required(
                    self.base.antagonist,
                    "an antagonist",
                    "waiting for an alerting soldier",
                );
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingRunningToOfficerSeen
                        | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                        | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                        | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                    ) => {
                        self.face_npc(self.base.antagonist, ctx);
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {
                        self.return_to_duty_default(env)?;
                    }
                }
            }
            StimulusType::CallReport => {
                unreachable!("report handoff must execute through the engine")
            }
            _ => {}
        }
        Ok(false)
    }

    fn seeking_officer_get_alerting_report_from_soldier(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        // Officer processes alerting report
        match stimulus_type {
            StimulusType::CallYourTalk1 => {
                unreachable!("alert report reply must execute through the engine")
            }
            StimulusType::EventMyTalk1 => {
                unreachable!("alert report reply must execute through the engine")
            }
            StimulusType::EventTimer
                if !self.alert_soldiers(
                    self.base.seek_position,
                    0,
                    env,
                    AlertSoldiersFailureContinuation::ReturnToDuty,
                )? =>
            {
                self.return_to_duty_default(env)?;
            }
            _ => {}
        }
        Ok(false)
    }

    // ============ ATTACKING ============

    fn seeking_knight_watching_tower_guard(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            // Knight reacts directly on alerts:
            //   search around seek_position with AI_HINT_SEEK_RADIUS, location first;
            self.seek_area(
                env,
                self.base.seek_position,
                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
                UNDEFINED_DIRECTION,
                global,
            )?;
        }
        Ok(false)
    }

    // Freeing someone from the net: wait out, or reach point
    // and take the net.

    fn seeking_net(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // If detected body is no longer stuck under net
                // AND I'm detecting them → resurrected,
                // return to duty; else re-arm timer. This is
                // cone-and-LOS detection, not the 360° feel
                // bubble, and it is short-circuited behind the net
                // check so a still-trapped body costs no LOS query.
                let body_stuck = ctx
                    .expect_entity_view(self.base.detected_body, "seeking-net body")
                    .stuck_under_net;
                if !body_stuck && self.is_detecting(self.base.detected_body, ctx) {
                    // Resurrected.
                    self.return_to_duty_default(env)?;
                } else {
                    self.base.launch_timer(10, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                // If detected body is still under net, riders just
                // search the area around themselves; foot units launch
                // the SEARCH×4+TAKE sequence + transition to
                // SeekingTakingNet. Otherwise return to duty.
                let body_stuck = ctx
                    .expect_entity_view(self.base.detected_body, "seeking-net body")
                    .stuck_under_net;
                if body_stuck {
                    if ctx.self_is_rider {
                        // Rider can't dismount to take the net;
                        // expand the seek radius and look.
                        let here = ctx.position;
                        self.seek_area(
                            env,
                            here,
                            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                            SeekFlags::BODY_SEEK,
                            UNDEFINED_DIRECTION,
                            global,
                        )?;
                    } else {
                        // SEARCH×4 + TAKE on interesting_object
                        // (the net).  Only fire the sequence if
                        // the object is still active.
                        if let Some(net_obj) = self.base.interesting_object
                            && ctx.entity_position(net_obj).is_some()
                        {
                            self.set_state(AiState::Seeking, Substate::SeekingTakingNet);
                            self.base.stop_all();
                            let owner = self.base.owner_entity_id;
                            let antagonist = Some(crate::element::EntityId::Net(
                                crate::entity_id::NetId(net_obj.get()),
                            ));
                            let mut seq = crate::sequence::Sequence::new();
                            seq.append_element(crate::sequence::SequenceElement::new_interaction(
                                1,
                                crate::element::Command::SearchCmd,
                                owner,
                                None,
                            ));
                            seq.append_element(crate::sequence::SequenceElement::new_interaction(
                                2,
                                crate::element::Command::SearchCmd,
                                owner,
                                None,
                            ));
                            seq.append_element(crate::sequence::SequenceElement::new_interaction(
                                3,
                                crate::element::Command::SearchCmd,
                                owner,
                                None,
                            ));
                            seq.append_element(crate::sequence::SequenceElement::new_interaction(
                                4,
                                crate::element::Command::SearchCmd,
                                owner,
                                None,
                            ));
                            seq.append_element(crate::sequence::SequenceElement::new_interaction(
                                5,
                                crate::element::Command::Take,
                                owner,
                                antagonist,
                            ));
                            self.base.outbox.actor.launch_sequences.push(seq);
                            self.base.set_emoticon(EmoticonType::None);
                        }
                    }
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Finished removing a net: free another, examine body,
    // or return to duty.

    // Officer scanning for free soldiers (three stages).

    fn seeking_officer_looking_for_soldiers1(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            let next = match self.base.current_substate {
                Substate::SeekingOfficerLookingForSoldiers1 => {
                    Substate::SeekingOfficerLookingForSoldiers1Sidewards
                }
                Substate::SeekingOfficerLookingForSoldiers2 => {
                    Substate::SeekingOfficerLookingForSoldiers2Sidewards
                }
                _ => Substate::SeekingOfficerLookingForSoldiers3Sidewards,
            };
            self.set_state(AiState::Seeking, next);
            self.base.outbox.actor.look_sidewards = Some(
                if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::OfficerSearchLook, 0..2) != 0 {
                    LookDirection::RightLeft
                } else {
                    LookDirection::LeftRight
                },
            );
        }
        false
    }

    // Sidewards 1/2 advance to next look stage, face 5/16
    // rotation, delay 30.

    fn seeking_officer_looking_for_soldiers1_sidewards(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            let next = match self.base.current_substate {
                Substate::SeekingOfficerLookingForSoldiers1Sidewards => {
                    Substate::SeekingOfficerLookingForSoldiers2
                }
                _ => Substate::SeekingOfficerLookingForSoldiers3,
            };
            self.set_state(AiState::Seeking, next);
            // Face (direction + 5) % 16.
            let new_dir = (ctx.direction + 5) % 16;
            self.base.face_direction(new_dir, ctx);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    // Stage-3 sidewards complete; done looking.

    fn seeking_officer_looking_for_soldiers3_sidewards(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // Charly search path: step through `search_charly_way`
    // on each reach point.

    fn seeking_charly(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventReachPoint {
            if !self.search_charly_way.is_empty() {
                self.search_charly_way.remove(0);
            }
            if self.search_charly_way.is_empty() {
                // If checkpoint_charly == 0 → return to duty;
                // else transition to CharlyWatching +
                // Look left and right.
                if self.base.checkpoint_charly.is_none() {
                    self.return_to_duty_default(env)?;
                } else {
                    self.set_state(AiState::Seeking, Substate::SeekingCharlyWatching);
                    self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                }
            } else {
                // Move to the next waypoint with RUN (+ DONT_STOP
                // if more than one remains).
                let next = self.search_charly_way[0];
                let flags = if self.search_charly_way.len() > 1 {
                    crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP
                } else {
                    crate::ai::GotoFlags::RUN
                };
                self.base.go_to(next, flags, ctx);
            }
        }
        Ok(false)
    }

    // Done watching at charly checkpoint: trigger
    // missed-charly alert.

    fn seeking_charly_watching(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.missed_charly_alert(env, global)?;
        }
        Ok(false)
    }

    // Detected charly reaction; rank-dependent follow-up.

    fn seeking_detected_charly(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            // Mark charly seen, then branch on rank.
            self.base.my_reconnaissance_report.charly_seen = true;
            match self.get_rank() {
                ProfileRank::Officer if !self.alerted_us.is_empty() => {
                    // Reload previous state + short timer.
                    // The reference leaves this branch as "tell
                    // all soldiers to go home" without an
                    // implementation; preserve the shipped
                    // reload-and-wait behavior.
                    let previous_state = self.previous_state.get("previous_state");
                    let previous_substate = self.previous_substate.get("previous_substate");
                    self.set_state_with_timer(previous_state, previous_substate, 10, ctx);
                }
                _ => {
                    // Soldier, Knight, or officer with no alerted
                    // soldiers all fall through to returning to duty.
                    self.return_to_duty_default(env)?;
                }
            }
        }
        Ok(false)
    }

    // Officer sends charly away toward another officer.

    fn seeking_send_charly_to_officer(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventMyTalk1 => {
                let charly = self.base.friend_in_trouble;
                let Some(charly) = charly else {
                    self.return_to_duty_default(env)?;
                    return Ok(false);
                };
                let antagonist = self.required(
                    self.base.antagonist,
                    "an antagonist",
                    "sending Charly to another officer",
                );
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RequestThinkResult {
                        target: charly.get(),
                        caller: self.base.me,
                        stimulus_type: StimulusType::CallGoToOfficer,
                        info: crate::ai::StimulusInfo::Human(antagonist),
                        continuation: ThinkResultContinuation::OfficerSentCharlyToOfficer,
                    },
                );
            }
            StimulusType::EventMyTalk2 => {
                self.set_state(AiState::Seeking, Substate::SeekingLookingResurrectedCharly);
                // The original game's report-to-officer state faces
                // the friend-in-trouble reference between state change and timer launch when
                // the second speech callback completes.
                self.base.face_entity(self.base.friend_in_trouble, ctx);
                self.base.launch_timer(100, ctx.frame);
            }
            _ => {}
        }
        Ok(false)
    }

    // Watching a charly who's been sent off; timer returns
    // to duty.

    fn seeking_looking_resurrected_charly(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // Charly was sent to officer; go near the officer.

    fn seeking_charly_sent_to_officer(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
            // Approach to within 40 units of the antagonist.
            let view =
                ctx.expect_entity_view(self.base.antagonist, "charly-sent-to-officer officer");
            self.base
                .go_near(view.position, 40, crate::ai::GotoFlags::empty(), ctx);
            // unalert_all_near_charly_seekers(me).
            // Drained engine-side via
            // `pending_unalert_near_charly_seekers` — the
            // engine walks all soldiers and dispatches
            // CallCharlyIsBack to ones detecting me 180°.
            self.base.outbox.actor.queue_unalert_near_charly_seekers(
                CharlySeekerTarget::SelfNpc,
                self.base.antagonist,
            );
            self.base.launch_timer(10, ctx.frame);
        }
        false
    }

    // Charly on the way to officer; timer either transitions
    // to "seen" or retries.

    fn seeking_charly_go_to_officer(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                let antagonist = self.required(
                    self.base.antagonist,
                    "an antagonist",
                    "reporting back to an officer",
                );
                // The original game checks whether this actor detects the antagonist. This is
                // the normal live view cone, not the 360° helper.
                if self.is_detecting(antagonist, ctx) {
                    // The engine delivers this action synchronously and
                    // feeds the officer's actual Think return value into
                    // `resolve_charly_officer_report`.
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::ReportBackToOfficer {
                            officer: antagonist.get(),
                            charly: self.base.me,
                        },
                    );
                } else {
                    // unalert_all_near_charly_seekers(me).
                    self.base.outbox.actor.queue_unalert_near_charly_seekers(
                        CharlySeekerTarget::SelfNpc,
                        self.base.antagonist,
                    );
                    self.base.launch_timer(10, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                self.return_to_duty_default(env)?;
            }
            _ => {}
        }
        Ok(false)
    }

    // Charly reached officer; reach→CallCoordinate; timer
    // keeps polling.

    fn seeking_charly_go_to_officer_seen(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // Only re-arm if antagonist is in
                // waiting for the checkpoint member; else return to duty.
                let waits_for_charly = ctx
                    .expect_entity_view(self.base.antagonist, "charly-go-to-officer officer")
                    .ai_substate
                    == Substate::SeekingOfficerWaitForCharly;
                if waits_for_charly {
                    // unalert_all_near_charly_seekers(me).
                    self.base.outbox.actor.queue_unalert_near_charly_seekers(
                        CharlySeekerTarget::SelfNpc,
                        self.base.antagonist,
                    );
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            StimulusType::EventReachPoint => {
                // antagonist.think(CallCoordinate, me)
                if let Some(antagonist) = self.base.antagonist {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallCoordinate,
                            info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                }
                self.set_state(AiState::Seeking, Substate::SeekingCharlyGetLectureByOfficer);
            }
            _ => {}
        }
        Ok(false)
    }

    // Charly receives the officer's lecture and transitions
    // to stage 2 on talk.

    fn seeking_charly_get_lecture_by_officer(&mut self, stimulus_type: StimulusType) -> bool {
        if stimulus_type == StimulusType::CallYourTalk1 {
            // Original tags Charly's defence as MYTALK_1. Its sound-finished
            // callback must therefore emit EventMyTalk1, which stage 2 relays
            // to the officer so the officer can speak the lecture's final
            // line (OfficerRebukesCharlyEnd).
            self.base
                .say_with_flags(Remark::CharlyDefendsHimself, SpeechFlags::MYTALK_1);
            self.set_state(
                AiState::Seeking,
                Substate::SeekingCharlyGetLectureByOfficer2,
            );
        }
        false
    }

    // Charly lecture stage 2: relays talk, ends on
    // CallYourTalk2.

    fn seeking_charly_get_lecture_by_officer2(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        match stimulus_type {
                StimulusType::EventMyTalk1
                    // antagonist.think(CallYourTalk1)
                    if self.base.antagonist.is_some() => {
                        let antagonist = self.required(self.base.antagonist, "an antagonist","answering an officer lecture");
                        self.base
                            .outbox.reentrant.cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: antagonist.get(),
                                stimulus_type: StimulusType::CallYourTalk1,
                                info: crate::ai::StimulusInfo::None,
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            });
                    }
                StimulusType::CallYourTalk2 => {
                    self.return_to_duty_default(env)?;
                }
                _ => {}
            }
        Ok(false)
    }

    // Officer waits for
    // charly: on timer inspect antagonist substate; on coordinate
    // call, rebuke.

    fn seeking_officer_wait_for_charly(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // If antagonist is still in one of the
                // "on the way" charly substates, face them, clear the
                // emoticon and re-arm the timer; else return to duty.
                let is_on_the_way = matches!(
                    ctx.expect_entity_view(self.base.antagonist, "officer-wait-for-charly")
                        .ai_substate,
                    Substate::SeekingCharlySentToOfficer
                        | Substate::SeekingCharlyGoToOfficer
                        | Substate::SeekingCharlyGoToOfficerSeen
                );
                if is_on_the_way {
                    self.base.face_entity(self.base.antagonist, ctx);
                    self.base.set_emoticon(EmoticonType::None);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            StimulusType::CallCoordinate => {
                // If antagonist == stimulus_info.human
                let human_matches = matches!(
                    stimulus.info,
                    crate::ai::StimulusInfo::Human(h) if Some(h) == self.base.antagonist,
                );
                if human_matches {
                    self.base.face_entity(self.base.antagonist, ctx);
                    self.base.say_with_flags(
                        Remark::OfficerRebukesCharly,
                        crate::ai::SpeechFlags::MYTALK_1,
                    );
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerLectureCharly);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Officer lectures charly. MyTalk1 → CallYourTalk1 to charly;
    // CallYourTalk1 → end lecture; MyTalk2 → point to best-waypoint
    // and launch pointing timer.

    fn seeking_officer_lecture_charly(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventMyTalk1 if self.base.antagonist.is_some() => {
                let antagonist =
                    self.required(self.base.antagonist, "an antagonist", "lecturing Charly");
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: crate::ai::StimulusInfo::None,
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                    });
            }
            StimulusType::CallYourTalk1 => {
                self.base.say_with_flags(
                    Remark::OfficerRebukesCharlyEnd,
                    crate::ai::SpeechFlags::MYTALK_2,
                );
            }
            StimulusType::EventMyTalk2 => {
                // If antagonist has a path, find the nearest waypoint to
                // the officer's position and point to it; else point to
                // the antagonist's initial position. The patrol-path
                // waypoint list isn't exposed on AiEntityView (only the
                // has_patrol_path flag) — when the antagonist has a path
                // we fall back to their current position as a reasonable
                // "where I want you to go" stand-in.  For the no-path
                // case we use `initial_position` which is now available
                // on the view.
                let view = ctx.expect_entity_view(self.base.antagonist, "officer-lecture charly");
                let target = if view.has_patrol_path {
                    // Best proxy without per-waypoint list.
                    view.position
                } else {
                    view.initial_position
                };
                self.base.point_to(target, ctx);
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingOfficerLectureCharlyPointing,
                );
                self.base.say_with_flags(
                    Remark::OfficerEndsConversation,
                    crate::ai::SpeechFlags::MYTALK_3,
                );
                self.base.launch_timer(20, ctx.frame);
            }
            _ => {}
        }
        false
    }

    // Pointing done: forward CALL_YOURTALK_2 and go home.

    fn seeking_officer_lecture_charly_pointing(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventMyTalk3 {
            if let Some(antagonist) = self.base.antagonist {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk2,
                        info: crate::ai::StimulusInfo::None,
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                    });
            }
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // Civilian reached soldier to report; waits for officer's
    // wait-state.

    fn seeking_civilian_running_to_soldier_seen(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventTimer => {
                // If antagonist is still waiting for an alerting civilian, re-arm
                // timer; else end.
                let officer_waiting = ctx
                    .expect_entity_view(self.base.antagonist, "civilian-report soldier")
                    .ai_substate
                    == Substate::SeekingWaitForAlertingCivilian;
                if officer_waiting {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            StimulusType::EventReachPoint => {
                // Only transition if antagonist still in
                // wait-for-alerting-civilian; else return to duty.
                let officer_waiting = ctx
                    .expect_entity_view(self.base.antagonist, "civilian-report soldier")
                    .ai_substate
                    == Substate::SeekingWaitForAlertingCivilian;
                if officer_waiting {
                    self.set_state_with_timer(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                        10,
                        ctx,
                    );
                } else {
                    self.return_to_duty_default(env)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Civilian begins report; denunciates and points.

    fn seeking_civilian_give_alerting_report_to_soldier_start(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(
                AiState::Seeking,
                Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
            );
            if let Some(antagonist) = self.base.antagonist {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallReport,
                        info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                    });
            }
            self.base.say(Remark::CivDenunciates);
            self.base.point_to(self.base.seek_position, ctx);
        }
        false
    }

    // Done pointing: transition to end and face antagonist.

    fn seeking_civilian_give_alerting_report_to_soldier_point(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(
                AiState::Seeking,
                Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
            );
            self.base.face_entity(self.base.antagonist, ctx);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    // Civilian panics after denunciation.

    fn seeking_civilian_give_alerting_report_to_soldier_end(
        &mut self,
        stimulus_type: StimulusType,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.panic_from_position(
                self.base.seek_position,
                parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
            );
        }
        false
    }

    /// Handle the original game's reach-point event while seeking an
    /// authored seek point. This is also called re-entrantly by the two
    /// passed-ambush substates, matching their direct `Think` call.
    fn reached_seek_point(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let ThinkEnv { sim, ctx, .. } = env;
        let seek_point_id = self
            .actual_seek_point
            .expect("seek-point arrival without an actual seek point");
        let directions = resolve_seek_point_id(
            seek_point_id,
            &self.personal_seek_point_1,
            &self.personal_seek_point_2,
            global,
        )
        .unwrap_or_else(|| panic!("actual seek point {seek_point_id:?} no longer resolves"))
        .directions
        .clone();

        self.seek_point_view_directions.clear();
        for direction in directions {
            // `(direction + 16 - current_direction) ^ 8` is the exact
            // precedence of the Original expression. Directions within one
            // sector of the direction the soldier arrived from are skipped.
            let relative = ((i32::from(direction) + 16 - i32::from(ctx.direction)) ^ 8) & 15;
            if matches!(relative, 15 | 0 | 1) {
                continue;
            }

            // The original game increments the count before random selection, so even
            // insertion into an empty list consumes one global RNG draw.
            let insertion = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::EnemySeekDirectionShuffle,
                0..=self.seek_point_view_directions.len(),
            );
            self.seek_point_view_directions.insert(insertion, direction);
        }

        if let Some(&direction) = self.seek_point_view_directions.first() {
            self.seek_point_view_directions.remove(0);
            self.set_state(AiState::Seeking, Substate::SeekingSeekpointWatching);
            self.base.face_direction(direction, ctx);
            self.base
                .launch_timer(parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32, ctx.frame);
        } else {
            self.seek_next_point(env, global)?;
        }
        Ok(())
    }

    fn think_expected_menacing_event(
        &mut self,
        stimulus: &Stimulus,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::MenacingPcInComa if stimulus_type == StimulusType::EventTimer => {
                // Require a PC within a maximum-axis distance of 100,
                // unconscious and in a coma. `is_pc` and `in_coma`
                // both live on `AiEntityView`, so the full triplet is
                // checkable here without approximation.
                let keep_watching = {
                    let v =
                        ctx.expect_entity_view(self.base.primary_target, "menacing-coma target");
                    // maximum-norm distance
                    // in the original game
                    // is the stretched **3D** Chebyshev distance between
                    // the two world positions, so a target
                    // 110 units above the guard is already out of range.
                    // A raw 2D map-space max-norm kept the soldier
                    // menacing a PC standing on a wall walkway forever.
                    let distance = ai_max_norm_distance(
                        &v.position,
                        v.elevation,
                        &ctx.position,
                        ctx.elevation,
                    );
                    v.is_pc && v.is_unconscious && v.in_coma && distance < 100.0
                };
                if keep_watching {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty_default(env)?;
                }
            }

            // Reached arrow reserves: refill ammo and search the surrounding area.
            _ => {}
        }
        Ok(false)
    }

    fn think_expected_fleeing_event(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::FleeingPanic | Substate::FleeingRunToHide | Substate::FleeingRunToDoor => {
                if self.base.current_substate == Substate::FleeingPanic
                    && matches!(
                        stimulus_type,
                        StimulusType::EventReachPoint | StimulusType::EventCouldntReachPoint
                    )
                    && self.base.lasting_panic_runs == 0
                {
                    // Malignity-specific: when the panic is spent,
                    // clear the seen-enemy counter so the soldier
                    // can re-spook on the next sighting.  Friendly
                    // AI doesn't track this counter the same way
                    // (its own reset lives elsewhere).
                    self.fleeing_seen_enemy_counter = 0;
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::FleeingHiding => {
                if stimulus_type == StimulusType::EventTimer {
                    // Original's shared AI handler ends the hiding interval
                    // through enemy-specific return-to-duty handling. The
                    // base Rust common handler cannot call back into its
                    // containing EnemyAi, so complete that response here.
                    //
                    // original-game behavior
                    self.return_to_duty_default(env)
                        .map_err(|duty| duty.with_think_result(true))?;
                    return Ok(true);
                }
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            // Merry man fleeing to map exit.
            Substate::FleeingMerryManRunToLeaveMap => {
                match stimulus_type {
                    StimulusType::EventTimer => {
                        // Stuck recovery: if we're not already sprinting
                        // toward the door, re-issue movement. Gated
                        // on `action_state != MovingFast` &&
                        // `last_goto_destination` set, so an actor
                        // mid-run doesn't get its sequence torn down
                        // every 30 frames.
                        let dest = self.base.last_goto_destination;
                        if ctx.self_action_state != crate::element::ActionState::MovingFast
                            && (dest.x != 0.0 || dest.y != 0.0)
                        {
                            self.base.stop_all();
                            self.go_to(
                                self.base.current_state,
                                self.base.current_substate,
                                dest,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                        }
                        self.base.launch_timer(30, ctx.frame);
                    }
                    StimulusType::EventReachPoint => {
                        // Check if we're near the door.
                        let dest = self.base.last_goto_destination;
                        let dx = ctx.position.x - dest.x;
                        let dy = ctx.position.y - dest.y;
                        let dist = dx.abs().max(dy.abs());
                        if dist < 10.0 {
                            // Arrived at the door entry — now run to the exit point
                            // to exit the map (launches a sequence
                            // element targeting the door's exit point).
                            self.set_state(AiState::Fleeing, Substate::FleeingMerryManLeaveMap);
                            if let Some(door_idx) = self.base.my_door_index {
                                // `my_door_index` is a global door-table
                                // index.  Find the matching reinforcement
                                // door entry (linear scan; small list)
                                // for the cached point_out geometry.
                                if let Some(door) = global
                                    .reinforcement_doors
                                    .iter()
                                    .find(|d| d.door_index == door_idx)
                                {
                                    let point_out_pos = Position {
                                        x: door.point_out.x,
                                        y: door.point_out.y,
                                        ..dest
                                    };
                                    self.base.run_to_map_exit(point_out_pos);
                                } else {
                                    // Door gone — just lock and deactivate.
                                    self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                                    self.base.outbox.actor.deactivate = true;
                                }
                            } else {
                                // No door stored — just lock and deactivate.
                                self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                                self.base.outbox.actor.deactivate = true;
                            }
                        } else {
                            // Not there yet — retry.
                            self.go_to(
                                self.base.current_state,
                                self.base.current_substate,
                                dest,
                                crate::ai::GotoFlags::RUN,
                                ctx,
                            );
                            self.base.launch_timer(30, ctx.frame);
                        }
                    }
                    _ => {}
                }
            }

            // Merry man has reached the exit point — deactivate.
            Substate::FleeingMerryManLeaveMap => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // `non_script_lock(Freeze); set_active(false);`
                    self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                    self.base.outbox.actor.deactivate = true;
                }
            }

            // Additional substate handlers
            //
            // The block below ports the ~83 substates that were
            // previously swept into the no-op group when the
            // exhaustive-match refactor landed.  Many of these arms
            // call helpers that were not yet implemented when this block first
            // landed. The remaining fallback arms below are kept explicit
            // and should be replaced with exact handlers as each parity
            // path is audited.

            // Empty case; script-driven
            // substate is handled elsewhere.  Kept here as an explicit arm to
            // document the mapping.
            Substate::FleeingRunForArrowReserves => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Flag the engine drain to refill the archer's arrows —
                    // the engine-side `pending_refill_bow_ammo` processor
                    // writes `NpcData::number_of_arrows = MAX_NPC_ARROWS`.
                    self.base.outbox.actor.refill_bow_ammo = true;
                    self.seek_area(
                        env,
                        self.base.seek_position,
                        parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                        global,
                    )?;
                }
            }

            // Run to alert soldiers: on reach, attempt alert; else fall
            // through hide.
            Substate::FleeingRunToAlertSoldiers => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // AlertSoldiers; on false → fall through to RUN_TO_DOOR
                    // and re-dispatch REACHPOINT.
                    //
                    // This substate is only reached via
                    // RunAndAlertSoldiers, which is officer-only ("an
                    // officer runs away and alerts soldiers").
                    // `alert_soldiers` debug-asserts Officer rank, matching
                    // that contract.
                    let seek_flags_bits = self.seek_flags.bits();
                    let alerted = self.alert_soldiers(
                        self.base.seek_position,
                        seek_flags_bits,
                        env,
                        AlertSoldiersFailureContinuation::FleeingRunToDoor,
                    )?;
                    if !alerted {
                        // Fire a self-stimulus so the re-delivery happens
                        // on the next think rather than recursing here
                        // (the original game's direct decision update is a
                        // self-recursion that the Rust state-machine
                        // contract avoids).
                        self.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);
                        self.base.fire_self_stimulus(StimulusType::EventReachPoint);
                    }
                }
            }

            // Retire from combat: reach → fast-turn toward seek position.
            Substate::FleeingRetireFromCombat => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Fleeing, Substate::FleeingRetireFromCombatTurn);
                    self.base
                        .face_position_with_ctx(self.base.seek_position, ctx);
                }
            }

            // Turned: if detecting target, make battle decisions; else overview.
            Substate::FleeingRetireFromCombatTurn if stimulus_type == StimulusType::EventDone => {
                if self.base.primary_target.is_some_and(|primary_target| {
                    self.is_detecting_180_degrees(primary_target, ctx)
                }) {
                    self.battle_decisions(env, global)?;
                } else {
                    self.get_battle_overview(0, env)?;
                }
            }

            // Sword fight step back reached point: resume swordfight with
            // 20-tick timer.
            _ => {}
        }
        Ok(false)
    }

    /// A near officer of my own camp who is already wondering about the
    /// same noise I am — i.e. an officer able to fight, not script-locked,
    /// within 360° detection range, whose live seek position is exactly
    /// mine.  Only a plain soldier ever asks this question.
    fn near_officer_who_is_wondering_about_the_same_noise(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> Option<NpcHandle> {
        debug_assert_eq!(self.get_rank(), ProfileRank::Soldier);
        let my_seek = self.base.seek_position;
        // The predicate order matters: the authoritative LOS query runs
        // before the script-lock and seek-position rejections, so its
        // synchronous cache side effect still happens for every candidate
        // that clears the rank / able-to-fight gate.
        let candidates: Vec<NpcHandle> = tick
            .camp_soldiers
            .iter()
            .filter(|cs| cs.rank == ProfileRank::Officer && cs.is_able_to_fight)
            .map(|cs| cs.handle)
            .collect();
        for handle in candidates {
            if !self.is_detecting_360_degrees(handle as HumanHandle, ctx) {
                continue;
            }
            let matches = tick.camp_soldiers.iter().any(|cs| {
                cs.handle == handle
                    && !cs.script_locked
                    && cs.seek_position.x == my_seek.x
                    && cs.seek_position.y == my_seek.y
                    && cs.seek_position.level == my_seek.level
            });
            if matches {
                return Some(handle);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
