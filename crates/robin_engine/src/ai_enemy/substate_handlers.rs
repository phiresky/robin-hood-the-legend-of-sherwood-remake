//! `EnemyAi::think_expected_event` — the substate state machine.
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the giant per-substate match
//! manageable. Lives in a separate `impl EnemyAi` block; child modules
//! see the parent's private fields and helpers.

mod attacking;

use crate::ai::*;
use crate::parameters_ai;

use super::util::{
    ai_max_norm_distance, ai_max_norm_distance_world, ai_square_distance, resolve_seek_point_id,
    vec_to_sector,
};
use super::{
    AlertSoldiersFailureContinuation, EnemyAi, PrimaryTargetFlags, ProfileRank, SeekFlags,
    UNDEFINED_DIRECTION, archer, combat, task_priority,
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

fn body_examination_target_disappeared(
    owner: crate::coordinates::WorldPoint3D,
    body: crate::coordinates::WorldPoint3D,
) -> bool {
    let delta = body - owner;
    delta.x.abs().max(delta.y.abs()).max(delta.z.abs())
        > (2 * parameters_ai::AI_STOP_BEFORE_BODY_STEPS) as f32
}

impl EnemyAi {
    /// Resolve the primary target inherited after reaching a phalanx slot.
    ///
    /// Original tests the left combat-neighbour pointer first, then the right,
    /// and copies that soldier's target verbatim. A present neighbour whose
    /// target is null therefore returns `Some(None)`; only the absence of both
    /// soldier neighbours returns outer `None` and authorizes
    /// `GetNewPrimaryTarget`.
    pub(super) fn phalanx_neighbour_primary_target(
        &self,
        tick: &AiPerTickData,
    ) -> Option<Option<AiEntityHandle>> {
        for neighbour in [self.left_combat_neighbour, self.right_combat_neighbour] {
            let Some(neighbour) = neighbour else {
                continue;
            };
            let fighter = self.find_fighter(neighbour.get(), tick).unwrap_or_else(|| {
                panic!(
                    "combat neighbour {neighbour} missing from complete fighter registry for {}",
                    self.base.me
                )
            });
            if fighter.is_soldier {
                return Some(fighter.primary_target);
            }
        }
        None
    }

    // One dispatcher preserves the numeric Substate machine while the
    // implementations are owned by coherent state families. See
    // original-code/RHartificialmalignity.cpp:ThinkExpectedEvent.
    pub(super) fn think_expected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        debug_assert_eq!(
            self.base.current_substate.ai_state_family(),
            Some(self.base.current_state),
            "EnemyAi expected-event dispatch received mismatched state/substate: {:?}/{:?}",
            self.base.current_state,
            self.base.current_substate
        );

        match self.base.current_state {
            AiState::Sleeping => {
                self.think_expected_sleeping_event(stimulus, global, ctx, tick, grid)
            }
            AiState::Default => {
                self.think_expected_default_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Wondering => {
                self.think_expected_wondering_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Seeking => {
                self.think_expected_seeking_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Attacking => {
                self.think_expected_attacking_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Menacing => {
                self.think_expected_menacing_event(sim, stimulus, global, ctx, tick, grid)
            }
            AiState::Fleeing => {
                self.think_expected_fleeing_event(sim, stimulus, global, ctx, tick, grid)
            }
        }
    }

    fn think_expected_sleeping_event(
        &mut self,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        _tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
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
            self.set_state(AiState::Wondering, Substate::WonderingLooking1);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    fn think_expected_default_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::DefaultGotoPost => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Original common handler calls virtual FaceTo followed
                    // by virtual SetState. For an enemy soldier that second
                    // call must reach EnemyAi::set_state so returning to
                    // Default queues LeaveAttentiveMode and ALERT_GREEN.
                    self.base
                        .face_direction(self.base.initial_view_direction, ctx);
                    self.set_state(AiState::Default, Substate::DefaultGotoPostTurn);
                    return true;
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
                    return true;
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
                // `think_expected_event_common_stuff` calls the virtual
                // `default_bored_standard_procedure` on timer expiry
                // during `DefaultOnPost`. Run it before delegating so
                // the subclass override takes effect; if it transitions
                // state we short-circuit, otherwise fall through to the
                // base timer.
                if self.base.current_substate == Substate::DefaultOnPost
                    && stimulus_type == StimulusType::EventTimer
                    && self.default_bored_standard_procedure(sim, ctx)
                {
                    return true;
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                    self.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
                    self.base.launch_timer(200, ctx.frame);
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
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        }
                    }
                }
            }

            Substate::DefaultGotoChief => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if let Some(patrol_chief) = self.base.patrol_chief {
                        // Original calls the element overload
                        // `Face(mpPatrolChief)`, which includes the chief's
                        // truncated elevation in the projection.
                        self.base.face_entity(patrol_chief.index(), ctx);
                        self.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
                        self.base.launch_timer(200, ctx.frame);
                    } else {
                        // Lost patrol chief — retry
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // ============ PATROL CHIEF RETURN ============
            Substate::DefaultPatrolChiefReturnToPatrol => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                        self.search_charly(sim, global, ctx, tick, grid);
                    }
                    self.base
                        .launch_timer(parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32, ctx.frame);
                }
            }

            // Done sweeping eyes, back to baseline looking for Charly.
            Substate::DefaultLookingSidewardsForCharly => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(AiState::Default, Substate::DefaultLookingForCharly);
                    self.base.launch_timer(10, ctx.frame);
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
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }

            // Synchronize with Charly; if he's gone astray, give up;
            // else wait for a SyncCharly event.
            Substate::DefaultSynchronizing => match stimulus_type {
                StimulusType::EventTimer => {
                    // If `synchronize_charly` is not in STATE_DEFAULT
                    // or is dead, return to duty; else re-arm the
                    // timer.
                    let sync_gone = ctx
                        .entity_view(self.base.synchronize_charly)
                        .map(|v| v.ai_state != AiState::Default || !v.is_able_to_fight)
                        .unwrap_or(true);
                    if sync_gone {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
        false
    }

    fn think_expected_wondering_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::WonderingWatching => self.wondering_watching(sim, stimulus_type, ctx, tick),

            Substate::WonderingLooking1 => self.wondering_looking1(sim, stimulus_type, ctx),

            Substate::WonderingLooking1Sidewards => {
                self.wondering_looking1_sidewards(sim, stimulus_type, ctx)
            }

            Substate::WonderingLooking2 => self.wondering_looking2(sim, stimulus_type, ctx),

            Substate::WonderingLooking2Sidewards => {
                self.wondering_looking2_sidewards(sim, stimulus_type, ctx)
            }

            Substate::WonderingMoneyReactiontime => {
                self.wondering_money_reactiontime(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingApproachingMoney => {
                self.wondering_approaching_money(stimulus_type, ctx, tick)
            }

            Substate::WonderingTakingMoney => self.wondering_taking_money(stimulus_type, ctx),

            Substate::WonderingWatchingForMoreMoney => {
                self.wondering_watching_for_more_money(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingAleReactiontime => {
                self.wondering_ale_reactiontime(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingApproachingAle => self.wondering_approaching_ale(stimulus_type, ctx),

            Substate::WonderingDrinkingAle => {
                self.wondering_drinking_ale(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingAppleSauceInTheVisor => {
                self.wondering_apple_sauce_in_the_visor(stimulus_type, ctx, tick)
            }

            Substate::WonderingHeardWhistling => self.wondering_heard_whistling(stimulus_type, ctx),

            Substate::WonderingWatchingTowerGuard => {
                self.wondering_watching_tower_guard(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingLooking3 => self.wondering_looking3(sim, stimulus_type),

            Substate::WonderingLooking3Sidewards => {
                self.wondering_looking3_sidewards(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingAppleReactiontime => {
                self.wondering_apple_reactiontime(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingAppleChasingChild => {
                self.wondering_apple_chasing_child(stimulus_type, ctx)
            }

            Substate::WonderingAppleChasingChildWaiting => {
                self.wondering_apple_chasing_child_waiting(stimulus_type, ctx)
            }

            Substate::WonderingAppleChasingChildEnd => {
                self.wondering_apple_chasing_child_end(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingRunningForMoney => {
                self.wondering_running_for_money(stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlReactiontime => {
                self.wondering_brawl_reactiontime(stimulus_type, ctx)
            }

            Substate::WonderingBrawlApproaching => {
                self.wondering_brawl_approaching(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlHitting => {
                self.wondering_brawl_hitting(stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlGotHit => {
                self.wondering_brawl_got_hit(stimulus_type, ctx, tick)
            }

            Substate::WonderingBrawlRecovering => {
                self.wondering_brawl_recovering(stimulus_type, ctx, tick)
            }

            Substate::WonderingApproachingToLoot => {
                self.wondering_approaching_to_loot(stimulus_type, ctx)
            }

            Substate::WonderingLooting => self.wondering_looting(sim, stimulus_type, ctx, tick),

            Substate::WonderingAleAway => self.wondering_ale_away(sim, stimulus_type, ctx, tick),

            Substate::WonderingOfficerSeeingBrawl => {
                self.wondering_officer_seeing_brawl(stimulus_type, ctx)
            }

            Substate::WonderingOfficerApproachingBrawl => {
                self.wondering_officer_approaching_brawl(stimulus_type, ctx, tick)
            }

            Substate::WonderingOfficerFinishingBrawl => {
                self.wondering_officer_finishing_brawl(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingOfficerFinishingBrawlWaiting => {
                self.wondering_officer_finishing_brawl_waiting(sim, stimulus_type, ctx, tick)
            }

            Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl => self
                .wondering_soldier_looking_officer_who_finished_brawl(
                    sim,
                    stimulus_type,
                    ctx,
                    tick,
                ),

            Substate::WonderingWatchingWhistling => {
                self.wondering_watching_whistling(sim, stimulus_type, global, ctx, tick)
            }

            Substate::WonderingApproachingBrawlVictim => {
                self.wondering_approaching_brawl_victim(stimulus_type)
            }

            Substate::WonderingAwakenBrawlVictim => {
                self.wondering_awaken_brawl_victim(sim, stimulus_type, ctx, tick)
            }

            // Attacker returns to another PC after menacing: begin a
            // swordfight.
            _ => false,
        }
    }

    fn wondering_watching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    fn wondering_looking1(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
    // FaceTo((dir+5)%16), launch_timer(30 + rand()&7).
    // Shared body for stages 1 & 2.

    fn wondering_looking1_sidewards(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
        sim: &crate::sim_rng::SimulationContext,
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
    // Looking3 (NOT ReturnToDuty), FaceTo((dir+5)%16),
    // 30 + rand()&7 timer.

    fn wondering_looking2_sidewards(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
    // (20-tick), Say(GoldYes), GoNear, 5-tick timer.
    // NO branch: CLOUD emoticon (50-tick), Say(GoldNo/VipGoldNo),
    // forget nearby coins, ReturnToDuty(KEEP_EMOTICON).

    fn wondering_money_reactiontime(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            let want_money = self.answer_question(Question::ShallITakeMoney, ctx);
            let obj_pos = ctx.entity_position(self.base.interesting_object);
            let officer_near = obj_pos
                .map(|p| self.is_any_angry_officer_near(p, tick))
                .unwrap_or(false);
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
                self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
            }
        }
        false
    }

    fn wondering_approaching_money(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Original shares one switch body between ApproachingMoney and
        // RunningForMoney. EVENT_TIMER refreshes the race; only
        // EVENT_REACHPOINT starts the Take interaction.
        self.wondering_running_for_money(stimulus_type, ctx, tick)
    }

    fn wondering_taking_money(&mut self, _stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        // Original intentionally ignores the expected-event type here.  The
        // Take sequence normally finishes with EVENT_DONE, but any expected
        // event advances the same completion boundary.
        if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
            self.base.interesting_object = Some(AiEntityHandle::new(coin));
            self.set_state(AiState::Wondering, Substate::WonderingMoneyReactiontime);
            self.base.launch_timer(1, ctx.frame);
        } else {
            self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
        }
        false
    }

    fn wondering_watching_for_more_money(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            // When the LookSidewards sequence finishes, scan for
            // nearby KO'd money-fight victims and either approach
            // to loot or return to duty.
            StimulusType::EventDone => {
                self.create_list_of_near_money_fight_victims(ctx, tick);

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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    // Ale reactiontime: if shall-take-ale:
    // stash beer as object_of_desire, transition to
    // ApproachingAle, set SUN emoticon (20-tick), Say(AleYes),
    // GoNear, save return point, 20-tick timer.  Otherwise
    // CLOUD emoticon (50-tick) + Say(AleNo / VipAleNo) +
    // ReturnToDuty with KEEP_EMOTICON.

    fn wondering_ale_reactiontime(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if self.answer_question(Question::ShallITakeAle, ctx) {
                assert!(
                    self.base.interesting_object.is_some(),
                    "ale reaction timer requires the retained bottle pointer"
                );
                // Original reads Position(mpInterestingObject) even when a
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
                self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
            }
        }
        false
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
                    self.set_state(AiState::Wondering, Substate::WonderingAleAway);
                    self.base.launch_timer(30, ctx.frame);
                } else {
                    self.base.launch_timer(20, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                if let Some(lost_pos) = self.is_beer_still_available(ctx) {
                    self.base.face_position_3d_with_ctx(lost_pos, ctx);
                    self.base.set_emoticon(EmoticonType::Thunderstorm);
                    self.set_state(AiState::Wondering, Substate::WonderingAleAway);
                    self.base.launch_timer(30, ctx.frame);
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    fn wondering_apple_sauce_in_the_visor(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // `get_angry_about_apple(seek_position)`: the
            // apple-origin position stashed when the apple
            // first landed.
            let pos = self.base.seek_position;
            self.get_angry_about_apple(&pos, ctx, tick);
        }
        false
    }

    // Heard whistling: face the source, transition to
    // WatchingWhistling, launch 60-tick timer.  The
    // decide-to-follow logic happens in the WatchingWhistling
    // timer arm — we just stage here.

    fn wondering_heard_whistling(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.base
                .face_position_3d_with_ctx(self.base.seek_position, ctx);
            self.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);
            self.base
                .launch_timer(parameters_ai::AI_FIRST_LOOK_TIME as u32, ctx.frame);
        }
        false
    }

    fn wondering_watching_tower_guard(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // ============ SEEKING ============

    // -- Seek-area substates --

    fn wondering_looking3(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
    ) -> bool {
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // Apple reaction: decide whether to chase, else return to duty.

    fn wondering_apple_reactiontime(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Logic:
            //   if !ShallIReactOnApple || !ChaseChilds()
            //     return_to_duty(sim, );
            //
            // ShallIReactOnApple outdoor answer:
            //   soldier_profile_apple > 0
            let shall_react = self.soldier_profile_apple > 0;
            let chased = shall_react && self.chase_childs(ctx);
            if !chased {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
    }

    // Chase child who threw the apple; panic-run counter
    // drives refreshes.

    fn wondering_apple_chasing_child(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        match stimulus_type {
                StimulusType::EventMyTalk1
                    // antagonist.think(CallYourTalk1)
                    if self.base.antagonist.is_some() => {
                        let antagonist = self.required_antagonist("chasing an apple-throwing child");
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
                StimulusType::EventTimer => {
                    if self.base.lasting_panic_runs > 0 {
                        self.base.lasting_panic_runs -= 1;
                        // Re-issue `go_near(sim, antagonist_pos, 5, RUN |
                        // DONT_STOP)` from the same substate each
                        // panic tick.  Our Shape 1 contract requires
                        // every movement name its new substate (see
                        // comment at fn go_near above), so route the
                        // refresh through
                        // `WonderingAppleChasingChildWaiting` — its
                        // EventTimer transitions back to ChasingChild.
                        // Issue GoNear from here bundled with the
                        // Waiting transition so the movement order
                        // lives with its new substate.
                        if let Some(view) = ctx.entity_view(self.base.antagonist) {
                            self.go_near(                                AiState::Wondering,
                                Substate::WonderingAppleChasingChildWaiting,
                                view.position,
                                5,
                                crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
                                ctx,
                            );
                        }
                        self.base.launch_timer(10, ctx.frame);
                    } else {
                        self.set_state(AiState::Wondering, Substate::WonderingAppleChasingChildEnd);
                        // Face(antagonist)
                        self.base.face_entity(self.base.antagonist, ctx);
                        self.base.launch_timer(30, ctx.frame);
                    }
                }
                StimulusType::EventReachPoint => {
                    self.set_state(
                        AiState::Wondering,
                        Substate::WonderingAppleChasingChildWaiting,
                    );
                    self.base.launch_timer(10, ctx.frame);
                }
                _ => {}
            }
        false
    }

    // Waiting between chase refreshes.

    fn wondering_apple_chasing_child_waiting(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Wondering, Substate::WonderingAppleChasingChild);
            self.base.launch_timer(1, ctx.frame);
        }
        false
    }

    // End of apple chase.

    fn wondering_apple_chasing_child_end(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
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
                    // GoNear(money, AI_STOP_BEFORE_MONEY_DISTANCE,
                    //         RUN | FIND_ACCESSIBLE)
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
                // (MaxNorm), take it + notify friends with
                // EventObjectAway; else look for more.
                let obj = self.base.interesting_object;
                let close_enough = ctx
                    .entity_position(obj)
                    .map(|p| {
                        let dx = (p.x - ctx.position.x).abs();
                        let dy = (p.y - ctx.position.y).abs();
                        dx.max(dy) < 25.0
                    })
                    .unwrap_or(false);
                if let Some(obj) = obj.filter(|_| close_enough) {
                    // StopAll + Take sequence.
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
            //   GoNear(seek_position, AI_HIT_DISTANCE, RUN);
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // If target moved > 3 units from the seek position,
                // update seek position and re-issue GoNear.
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
                let friend = self.required_friend_in_trouble("reaching a brawl opponent");
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
        false
    }

    // Brawl hit resolution; civilians panic, chase chain continues.

    fn wondering_brawl_hitting(
        &mut self,
        stimulus_type: StimulusType,
        _ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // The owner-work continuation performs the civilian sweep,
            // synchronously settles the later officer notification, then
            // invokes the remaining brawler tail. Keeping the tail out of
            // this initial outbox is essential because owner work otherwise
            // drains ahead of cross-NPC calls.
            self.base.completion_latch_inside_think = true;
            self.base.outbox.reentrant.brawl_hitting_completion_pending = true;
            self.nearby_civilians_panic_180();
        }
        false
    }

    pub(crate) fn brawl_hitting_notify_officer(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        self.maybe_officer_sees_me_fighting(ctx, tick);
    }

    pub(crate) fn resume_brawl_hitting_after_officer(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Remove KO'd target from the enemy list.
        if let Some(friend_in_trouble) = self.base.friend_in_trouble {
            let fit = friend_in_trouble.get();
            let is_unconscious = ctx
                .expect_entity_view(fit as HumanHandle, "brawl-hitting friend")
                .is_unconscious;
            if is_unconscious {
                self.money_fight_enemies.retain(|h| *h != fit);
            }
        }

        // Refresh the enemy list if we've run out —
        // picks up any same-camp soldier that joined the
        // brawl after our initial snapshot.
        if self.money_fight_enemies.is_empty() {
            self.create_new_list_of_money_fight_enemies(tick, ctx);
        }

        // Morale-gated continue-or-stop.
        if !self.wants_to_continue_money_fight(tick, ctx) {
            self.money_fight_enemies.clear();
            // stop_brawling_and_collect_money().
            self.stop_brawling_and_collect_money(ctx, tick);
        } else {
            // Handle 0 ("no brawl partner") deliberately
            // fails this gate and falls through to picking
            // the nearest remaining enemy.
            let fit_ok = self.base.friend_in_trouble.is_some_and(|friend| {
                !ctx.expect_entity_view(friend, "brawl-hitting friend")
                    .is_unconscious
            });
            if fit_ok {
                self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
                self.base.face_entity(self.base.friend_in_trouble, ctx);
                self.base.launch_timer(30, ctx.frame);
            } else if let Some(next) = self.get_nearest_money_fight_enemy(ctx) {
                self.base.friend_in_trouble = Some(AiEntityHandle::new(next));
                self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
                self.base.launch_timer(10, ctx.frame);
            } else {
                // stop_brawling_and_collect_money().
                self.stop_brawling_and_collect_money(ctx, tick);
            }
        }
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
            // SetEmoticon(Thunderstorm).
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
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // Pick nearest money-fight enemy and approach.
            if let Some(next) = self.get_nearest_money_fight_enemy(ctx) {
                self.base.friend_in_trouble = Some(AiEntityHandle::new(next));
                self.set_state(AiState::Wondering, Substate::WonderingBrawlApproaching);
                let view = ctx.expect_entity_view(next as HumanHandle, "brawl-recovering enemy");
                self.base.go_near(
                    view.position,
                    parameters_ai::AI_HIT_DISTANCE,
                    crate::ai::GotoFlags::RUN,
                    ctx,
                );
                // maybe_officer_sees_me_fighting().
                self.maybe_officer_sees_me_fighting(ctx, tick);
            } else {
                // stop_brawling_and_collect_money().
                self.stop_brawling_and_collect_money(ctx, tick);
            }
        }
        false
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
            let body = self.required_detected_body("reacting to a body");
            let v = ctx.expect_entity_view(body, "loot-approach body");
            let body_pos = v.position;
            let is_tied = v.posture == crate::element::Posture::Tied;
            let dx = body_pos.x - ctx.position.x;
            let dy = body_pos.y - ctx.position.y;
            let dist = dx.abs().max(dy.abs());
            if dist > 100.0 {
                // Too far — let Looting handle re-entry.
                self.set_state(AiState::Wondering, Substate::WonderingLooting);
                // Kick the state machine via a 1-tick timer;
                // the Looting arm handles the follow-up.  We
                // can't re-enter `think()` from inside an arm,
                // so fall back to a short timer that reaches
                // the same code path.
                self.base.launch_timer(1, ctx.frame);
            } else if is_tied {
                // Spot the tied body and transition to
                // body-seek; emit the reconnaissance report
                // update.
                self.base.my_reconnaissance_report.add_seen_body(body.get());
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Body, body_pos);
                self.set_state(AiState::Seeking, Substate::SeekingBody);
                // Re-issue Think(EventReachPoint) via a 1-tick
                // timer (see comment above).
                self.base.launch_timer(1, ctx.frame);
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
                self.return_to_duty(sim, DutyFlags::KEEP_EMOTICON, ctx, tick);
            }
        }
        false
    }

    // Beer went away: try next remembered beer, else return to duty.

    fn wondering_ale_away(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if !self.other_seen_ale.is_empty() {
                // Remember next beer as object of desire.
                let next = self.other_seen_ale.remove(0);
                let next = AiEntityHandle::new(next);
                self.base.object_of_desire = Some(next);
                self.base.interesting_object = Some(next);
                // SetState(Wondering, ApproachingAle).
                self.set_state(AiState::Wondering, Substate::WonderingApproachingAle);
                // GoNear(obj_pos, AI_STOP_BEFORE_MONEY_DISTANCE, FIND_ACCESSIBLE)
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
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
            // GoNear(friend_in_trouble.position, 100);
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
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                // If already talking, delay finish.
                if self.base.current_remark != Remark::TheSoundOfSilence {
                    self.base.launch_timer(50, ctx.frame);
                } else {
                    self.begin_finishing_brawl(ctx, tick);
                }
            }
            StimulusType::EventTimer => {
                self.begin_finishing_brawl(ctx, tick);
            }
            _ => {}
        }
        false
    }

    // Finishing-brawl orchestration: chain CallYourTalk1..3,
    // then timer dismisses soldiers and waits on the antagonist.

    fn wondering_officer_finishing_brawl(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer | StimulusType::EventMyTalk2 => {
                // forget_all_nearby_coins().
                self.forget_all_nearby_coins(ctx);
                // Walk list_us, send ReturnToDuty to each soldier
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
                    self.set_state(
                        AiState::Wondering,
                        Substate::WonderingOfficerFinishingBrawlWaiting,
                    );
                    self.base.launch_timer(10, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::CallYourTalk3 => {
                self.base.say(Remark::OfficerEndsConversation);
            }
            _ => {}
        }
        false
    }

    // Keep waiting while antagonist still
    // approaching/awakening a victim; else end.

    fn wondering_officer_finishing_brawl_waiting(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
    }

    // Soldier side of the "officer finished brawl" lecture:
    // 3-variant excuse speeches until the timer fires.

    fn wondering_soldier_looking_officer_who_finished_brawl(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // forget_all_nearby_coins(); return_to_duty(sim, );
                self.forget_all_nearby_coins(ctx);
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
        false
    }

    // Listening after whistling sound: decide whether to
    // investigate or bail.

    fn wondering_watching_whistling(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // ShallIFollowWhistle outdoor arm:
            //   whistle > 1 && company_number != 100
            let shall_follow = self.soldier_profile_whistle > 1 && self.company_number != 100;
            if !shall_follow {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                return false;
            }

            // Rank branch.
            let rank = self.get_rank();
            let mut near_officer: Option<NpcHandle> = None;
            let mut look_for_soldiers = false;
            match rank {
                ProfileRank::Soldier => {
                    near_officer =
                        self.near_officer_who_is_wondering_about_the_same_noise(ctx, tick);
                }
                ProfileRank::Officer => {
                    // ShallISendOutSoldier outdoor arm:
                    //   initiative < 50 || patrol.len() > 0
                    look_for_soldiers =
                        self.soldier_profile_initiative < 50 || !self.base.patrol.is_empty();
                }
                ProfileRank::Knight | ProfileRank::None => {}
            }

            if let Some(officer) = near_officer {
                // Face officer + transition to
                // DefaultLookingOfficerForAdvice + ? emoticon
                // + 100-tick timer.
                self.base.face_entity(officer, ctx);
                self.set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.base.launch_timer(100, ctx.frame);
            } else if look_for_soldiers {
                // OfficerLookForSoldier(ReportType::Noise).
                self.officer_look_for_soldier(ReportType::Noise, ctx, tick);
            } else {
                // SeekArea(seek_position,
                //   (MAX_WHISTLE_SEEK_RADIUS * (whistle - 2)) / 98,
                //   LOCATION_FIRST | WALKING);
                const MAX_WHISTLE_SEEK_RADIUS: u32 = 400;
                let whistle = self.soldier_profile_whistle as u32;
                let radius = if whistle >= 2 {
                    ((MAX_WHISTLE_SEEK_RADIUS * (whistle - 2)) / 98) as u16
                } else {
                    0
                };
                self.seek_area(
                    sim,
                    self.base.seek_position,
                    radius,
                    SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
        }
        false
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.awake_next_money_fight_victim_if_any(sim, ctx, tick);
        }
        false
    }

    fn think_expected_seeking_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::SeekingSeekpoint => {
                self.seeking_seekpoint(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingSeekpointWatching => {
                self.seeking_seekpoint_watching(sim, stimulus_type)
            }

            Substate::SeekingSeekpointWatchingSidewards => {
                self.seeking_seekpoint_watching_sidewards(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingSeekpointPassedAmbushPointLeft => self
                .seeking_seekpoint_passed_ambush_point_left(sim, stimulus_type, global, ctx, tick),

            Substate::SeekingSeekpointPassedAmbushPointRight => self
                .seeking_seekpoint_passed_ambush_point_right(sim, stimulus_type, global, ctx, tick),

            Substate::SeekingSeekpointCheckingAmbushPoint => {
                self.seeking_seekpoint_checking_ambush_point(stimulus_type, global, ctx)
            }

            Substate::SeekingSeekpointApproachingBeggar => {
                self.seeking_seekpoint_approaching_beggar(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingSeekpointIdentifyingBeggar1 => {
                self.seeking_seekpoint_identifying_beggar1(stimulus_type, ctx, tick)
            }

            Substate::SeekingSeekpointIdentifyingBeggar2 => {
                self.seeking_seekpoint_identifying_beggar2(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingHeardstepsPreReactiontime => {
                self.seeking_heardsteps_pre_reactiontime(stimulus_type, ctx, tick)
            }

            Substate::SeekingHeardstepsReactiontime => {
                self.seeking_heardsteps_reactiontime(stimulus_type, ctx, tick)
            }

            Substate::SeekingHeardsteps => {
                self.seeking_heardsteps(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingJustWatching => self.seeking_just_watching(sim, stimulus_type, ctx),

            Substate::SeekingJustWatchingSidewards => {
                self.seeking_just_watching_sidewards(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingBodyReactiontime => {
                self.seeking_body_reactiontime(stimulus_type, ctx, tick, grid)
            }

            Substate::SeekingBody => self.seeking_body(sim, stimulus_type, global, ctx, tick, grid),

            Substate::SeekingBodyLookingDeadBody => {
                self.seeking_body_looking_dead_body(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::SeekingBodyAwakeningSleeperr => {
                self.seeking_body_awakening_sleeperr(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::SeekingArrowReactiontime => {
                self.seeking_arrow_reactiontime(stimulus_type, ctx)
            }

            Substate::SeekingArrow => self.seeking_arrow(sim, stimulus_type, global, ctx, tick),

            Substate::SeekingArrowJustWatching | Substate::SeekingArrowJustWatchingSidewards => {
                self.seeking_arrow_just_watching(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::SeekingCombatAlertReactiontime => {
                self.seeking_combat_alert_reactiontime(stimulus_type, ctx)
            }

            Substate::SeekingCombatAlert => {
                self.seeking_combat_alert(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingGotStopEvent => {
                self.seeking_got_stop_event(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingWaitForAlertingCivilian => {
                self.seeking_wait_for_alerting_civilian(sim, stimulus, stimulus_type, ctx, tick)
            }

            Substate::SeekingGetReportFromCivilian => {
                self.seeking_get_report_from_civilian(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingGetAlertingReportFromCivilian => {
                self.seeking_get_alerting_report_from_civilian(stimulus_type, ctx)
            }

            Substate::SeekingGetAlertingReportFromCivilianLook => self
                .seeking_get_alerting_report_from_civilian_look(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::SeekingOfficerCallSoldier => self.seeking_officer_call_soldier(stimulus_type),

            Substate::SeekingOfficerWaitForSoldier => {
                self.seeking_officer_wait_for_soldier(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerInstructSoldier => {
                self.seeking_officer_instruct_soldier(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerWaitForInstructedSoldier => self
                .seeking_officer_wait_for_instructed_soldier(
                    sim,
                    stimulus,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::SeekingOfficerGetReportFromSoldier => {
                self.seeking_officer_get_report_from_soldier(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierCalledByOfficer => {
                self.seeking_soldier_called_by_officer(stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierGoToOfficer => {
                self.seeking_soldier_go_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierGetInstructedByOfficer => self
                .seeking_soldier_get_instructed_by_officer(sim, stimulus_type, global, ctx, tick),

            Substate::SeekingSoldierReturnToOfficer => {
                self.seeking_soldier_return_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierGiveReportToOfficer => {
                self.seeking_soldier_give_report_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerCallGroup => {
                self.seeking_officer_call_group(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::SeekingOfficerWaitForGroup => {
                self.seeking_officer_wait_for_group(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerInstructGroup => {
                self.seeking_officer_instruct_group(stimulus_type, ctx)
            }

            Substate::SeekingOfficerInstructGroupPointing => {
                self.seeking_officer_instruct_group_pointing(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerWaitForInstructedGroup => self
                .seeking_officer_wait_for_instructed_group(sim, stimulus, stimulus_type, ctx, tick),

            Substate::SeekingOfficerWaitInsideHouseToInstructGroup => {
                self.seeking_officer_wait_inside_house_to_instruct_group(stimulus_type, ctx)
            }

            Substate::SeekingOfficerLeavingHouseToInstructGroup => {
                self.seeking_officer_leaving_house_to_instruct_group(stimulus_type, ctx)
            }

            Substate::SeekingGroupCalledByOfficer => {
                self.seeking_group_called_by_officer(stimulus_type, ctx, tick)
            }

            Substate::SeekingGroupGoToOfficer => {
                self.seeking_group_go_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingGroupGetInstructedByOfficer => self
                .seeking_group_get_instructed_by_officer(
                    sim,
                    stimulus,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                ),

            Substate::SeekingRunningToOfficer => {
                self.seeking_running_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingRunningToOfficerSeen => {
                self.seeking_running_to_officer_seen(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerStart => {
                self.seeking_soldier_give_alerting_report_to_officer_start(stimulus_type, ctx, tick)
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerPoint => {
                self.seeking_soldier_give_alerting_report_to_officer_point(stimulus_type, ctx)
            }

            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd => self
                .seeking_soldier_give_alerting_report_to_officer_end(sim, stimulus_type, ctx, tick),

            Substate::SeekingOfficerWaitForAlertingSoldier => self
                .seeking_officer_wait_for_alerting_soldier(sim, stimulus, stimulus_type, ctx, tick),

            Substate::SeekingOfficerGetAlertingReportFromSoldier => self
                .seeking_officer_get_alerting_report_from_soldier(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::SeekingKnightWatchingTowerGuard => {
                self.seeking_knight_watching_tower_guard(sim, stimulus_type, global, ctx, tick)
            }

            Substate::SeekingNet => self.seeking_net(sim, stimulus_type, global, ctx, tick),

            Substate::SeekingTakingNet => {
                self.seeking_taking_net(sim, stimulus_type, ctx, tick, grid)
            }

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
                self.seeking_officer_looking_for_soldiers3_sidewards(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingCharly => self.seeking_charly(sim, stimulus_type, ctx, tick),

            Substate::SeekingCharlyWatching => {
                self.seeking_charly_watching(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::SeekingDetectedCharly => {
                self.seeking_detected_charly(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingSendCharlyToOfficer => {
                self.seeking_send_charly_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingLookingResurrectedCharly => {
                self.seeking_looking_resurrected_charly(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingCharlySentToOfficer => {
                self.seeking_charly_sent_to_officer(stimulus_type, ctx)
            }

            Substate::SeekingCharlyGoToOfficer => {
                self.seeking_charly_go_to_officer(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingCharlyGoToOfficerSeen => {
                self.seeking_charly_go_to_officer_seen(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingCharlyGetLectureByOfficer => {
                self.seeking_charly_get_lecture_by_officer(stimulus_type)
            }

            Substate::SeekingCharlyGetLectureByOfficer2 => {
                self.seeking_charly_get_lecture_by_officer2(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerWaitForCharly => {
                self.seeking_officer_wait_for_charly(sim, stimulus, stimulus_type, ctx, tick)
            }

            Substate::SeekingOfficerLectureCharly => {
                self.seeking_officer_lecture_charly(stimulus_type, ctx)
            }

            Substate::SeekingOfficerLectureCharlyPointing => {
                self.seeking_officer_lecture_charly_pointing(sim, stimulus_type, ctx, tick)
            }

            Substate::SeekingCivilianRunningToSoldierSeen => {
                self.seeking_civilian_running_to_soldier_seen(sim, stimulus_type, ctx, tick)
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

            // Reserve overview re-evaluates via BattleDecisions.
            _ => false,
        }
    }

    fn seeking_seekpoint(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint && self.actual_seek_point.is_some() {
            self.reached_seek_point(sim, global, ctx, tick);
        }
        false
    }

    fn seeking_seekpoint_watching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone || stimulus_type == StimulusType::EventTimer {
            // Check if more directions to look
            if let Some(&dir) = self.seek_point_view_directions.first() {
                self.seek_point_view_directions.remove(0);
                self.base.face_direction(dir, ctx);
                self.base.number_of_looks = 0;
                self.set_state(AiState::Seeking, Substate::SeekingSeekpointWatching);
                self.base
                    .launch_timer(parameters_ai::AI_SEEKPOINT_LOOK_TIME as u32, ctx.frame);
            } else {
                // No directions left — move to next seek point
                self.seek_next_point(sim, global, ctx, tick);
            }
        }
        false
    }

    fn seeking_seekpoint_passed_ambush_point_left(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                // Original calls Think(EVENT_REACHPOINT) re-entrantly
                // here; do the same work inline instead of synthesizing
                // a one-frame timer.
                if self.actual_seek_point.is_some() {
                    self.reached_seek_point(sim, global, ctx, tick);
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
        false
    }

    fn seeking_seekpoint_passed_ambush_point_right(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.set_state(AiState::Seeking, Substate::SeekingSeekpoint);
                if self.actual_seek_point.is_some() {
                    self.reached_seek_point(sim, global, ctx, tick);
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
        false
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier is walking toward the beggar's last known
        // position (set by seek_next_point → go_near).
        // On arrival, stop and begin identification.
        if stimulus_type == StimulusType::EventReachPoint {
            let beggar = self.required_beggar_to_examine("approaching a beggar seek point");
            // `RHartificialmalignity.cpp:2394-2427`: the arrival is only an
            // identification when `MaxNormDistance( mpBeggarToExamine ) < 100`.
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
                self.seek_next_point(sim, global, ctx, tick);
                return false;
            }
            self.base.stop_all();
            self.set_state(
                AiState::Seeking,
                Substate::SeekingSeekpointIdentifyingBeggar1,
            );
            self.base.say(Remark::ControlsBeggar);

            // Original authors one two-level sequence, not two
            // independent LaunchSequenceElement calls: TurnFast must
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
                // Original calls `mpBeggarToExamine->IsNPC()` here instead
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
        false
    }

    fn seeking_seekpoint_identifying_beggar1(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // First inspection phase: timer fires after the
        // menace/equip-bow animation completes.
        if stimulus_type == StimulusType::EventTimer {
            let beggar = self.required_beggar_to_examine("identifying a beggar");
            // `RHartificialmalignity.cpp` queries
            // `mpBeggarToExamine->IsNPC()` at the instant this timer fires.
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
                // with `beggar->Say(CIV_REMARK_BEGGAR_IDENTIFIES_HIMSELF)`.
                // Keep both calls in the ordered actor-effect prefix:
                // SetState below snapshots that prefix before its
                // synchronous script callback.
                self.base
                    .outbox
                    .actor
                    .say_on_target
                    .push((beggar, crate::ai::Remark::CivBeggarIdentifiesHimself));
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointIdentifyingBeggar2,
                );
                self.base.launch_timer(50, ctx.frame);
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
                    self.shoot_arrow_at(beggar.get(), ctx, tick);
                } else {
                    // Melee: call PC for duel.
                    self.begin_swordfight(ctx, tick);
                }
            }
        }
        false
    }

    fn seeking_seekpoint_identifying_beggar2(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Second phase (NPC path only): the real beggar has
        // identified themselves. Timer fires → resume seeking.
        if stimulus_type == StimulusType::EventTimer {
            self.seek_next_point(sim, global, ctx, tick);
        }
        false
    }

    // Pre-reactiontime gates whether to investigate himself
    // or just watch:
    //   - SOLDIER/KNIGHT: AnswerQuestion(ShallIFollowSteps)
    //   - OFFICER: only if no patrol *and* close enough to noise.
    // If "do not investigate yourself" → JustWatching, else
    // HeardstepsReactiontime.  Both arms set Q-mark + face + 60-tick
    // timer.

    fn seeking_heardsteps_pre_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            let do_not_investigate = match self.get_rank() {
                ProfileRank::Officer => {
                    // Original checks `mlistPatrol`, the officer's current
                    // patrol, not whether any camp snapshot still names this
                    // officer as chief. A separated member lives in
                    // `mlistMissedPatrolMembers` and must not stop the officer
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
                // Original's switch has no RANK_NONE arm and leaves its local
                // `bDoNotInvestigateYourself` unchanged.  The shipped Linux
                // v48 build's stable result is false here (observed for the
                // same inactive rank-less soldier in Savegame_023 replays 004
                // and 005), so make that binary behaviour defined instead of
                // incorrectly folding RANK_NONE into the soldier question.
                ProfileRank::None => false,
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
                self.set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                self.base.launch_timer(100, ctx.frame);
            } else {
                self.go_to(
                    AiState::Seeking,
                    Substate::SeekingHeardsteps,
                    self.base.seek_position,
                    GotoFlags::empty(),
                    ctx,
                );
                self.base.launch_timer(200, ctx.frame);
            }
        }
        false
    }

    fn seeking_heardsteps(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint | StimulusType::EventTimer => {
                // Search exactly at the noise source. Original uses
                // the actor's live position, not the remembered noise
                // position, and creates one personal seek point with
                // random look directions before walking to it.
                self.seek_area(
                    sim,
                    ctx.position,
                    0,
                    SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            _ => {}
        }
        false
    }

    fn seeking_just_watching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // Original has explicit SOLDIER and OFFICER arms and no
            // default arm. A knight can nevertheless inherit this
            // substate from a legacy save, in which case EVENT_DONE
            // deliberately leaves it unchanged.
            match self.get_rank() {
                ProfileRank::Soldier => {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
                ProfileRank::Officer => {
                    self.officer_look_for_soldier(ReportType::Noise, ctx, tick);
                }
                ProfileRank::Knight | ProfileRank::None => {}
            }
        }
        false
    }

    fn seeking_body_reactiontime(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Body-reactiontime expiry branches on rank:
            //   SOLDIER → look for a near officer who's already informed
            //             about this body (and stand by for instructions)
            //   OFFICER → if body is far enough, delegate; else examine
            //   KNIGHT  → examine themselves
            let body = self.required_detected_body("reacting to a body");
            let mut nearby_officer: Option<NpcHandle> = None;
            let mut look_for_soldiers = false;

            match self.get_rank() {
                ProfileRank::Soldier => {
                    // `near_officer_who_is_informed_about_this_body(body)`.
                    // Find the first same-camp officer who is
                    // able-to-fight, not script-locked, that
                    // we can detect 360°, and who has dropped
                    // `body` from their DETECTABLE_BODY list
                    // (i.e. has already processed it).  This
                    // uses the officer's live detectable-body
                    // snapshot in
                    // `CampSoldierInfo::detectable_bodies`.
                    nearby_officer = tick.camp_soldiers.iter().find_map(|cs| {
                        if cs.rank != ProfileRank::Officer || !cs.is_able_to_fight {
                            return None;
                        }
                        // The LOS query sits third in the
                        // predicate chain, ahead of the
                        // script-lock and detectable-list
                        // rejections, and its visibility-cache
                        // side effect happens for every officer
                        // that reaches this point.
                        if !self.is_detecting_360_degrees(cs.handle, ctx) {
                            return None;
                        }
                        if cs.script_locked {
                            return None;
                        }
                        // Officer must have already processed
                        // this body (dropped it from their
                        // DETECTABLE_BODY list).
                        if cs.detectable_bodies.contains(&body.get()) {
                            return None;
                        }
                        Some(cs.handle)
                    });
                }
                ProfileRank::Officer => {
                    // If body is further than
                    // OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE,
                    // delegate via ShallISendOutSoldier.
                    let body_view = ctx.entity_view(body).unwrap_or_else(|| {
                        panic!(
                            "officer {} cannot react to missing body {}",
                            self.base.me, body
                        )
                    });
                    let dist = ai_max_norm_distance(
                        &body_view.position,
                        body_view.elevation,
                        &ctx.position,
                        ctx.elevation,
                    );
                    if dist > combat::OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE as f32 {
                        look_for_soldiers =
                            self.answer_question(Question::ShallISendOutSoldier, ctx);
                    }
                }
                ProfileRank::Knight | ProfileRank::None => {}
            }

            if let Some(off) = nearby_officer {
                // Face + go into DefaultLookingOfficerForAdvice.
                self.base.face_entity(off, ctx);
                self.set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.base.launch_timer(100, ctx.frame);
            } else if look_for_soldiers {
                // OfficerLookForSoldier(ReportType::Body).
                self.officer_look_for_soldier(ReportType::Body, ctx, tick);
            } else {
                // RunToExamineBody(body).
                self.run_to_examine_body(body.get(), ctx, tick, grid);
            }
        }
        false
    }

    fn seeking_body(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // The timer only watches for a body that has recovered
            // while we are travelling. Body examination itself is
            // exclusively driven by EVENT_REACHPOINT in the original.
            let body_handle = self.required_detected_body("travelling toward a body");
            let view = ctx.entity_view(body_handle).unwrap_or_else(|| {
                panic!("SeekingBody timer target {body_handle} has no typed live entity view")
            });
            if !view.is_dead && !view.is_unconscious && self.is_detecting(body_handle, ctx) {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            } else {
                self.base.launch_timer(10, ctx.frame);
            }
        } else if stimulus_type == StimulusType::EventReachPoint {
            let body_handle = self.required_detected_body("reaching a body");
            let view = ctx.entity_view(body_handle).unwrap_or_else(|| {
                panic!("SeekingBody target {body_handle} has no typed live entity view")
            });
            let owner_world = ctx
                .entity_view(self.base.me)
                .unwrap_or_else(|| {
                    panic!(
                        "SeekingBody owner {} has no typed live entity view",
                        self.base.me
                    )
                })
                .detection_position_world;

            self.base.set_emoticon(EmoticonType::XMark);
            if body_examination_target_disappeared(owner_world, view.detection_position_world) {
                // `GoNear` can report REACHPOINT at its authored
                // tolerance even though the body's live element
                // position is no longer near the actor. Original
                // rejects that stale arrival before classifying the
                // body as dead/tied and starts a local body search.
                if !self.examine_other_bodies(ctx, tick) {
                    if view.is_unconscious || view.is_dead {
                        self.base.outbox.actor.add_detectables.push((
                            ctx.entity_id(body_handle).unwrap_or_else(|| {
                                panic!("SeekingBody target {body_handle} has no typed entity id")
                            }),
                            crate::element::DetectableType::Body,
                        ));
                    }
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    let seek_flags = SeekFlags::LOCATION_FIRST | self.seek_flags;
                    match self.get_rank() {
                        ProfileRank::Soldier | ProfileRank::Knight | ProfileRank::None => {
                            self.seek_area(
                                sim,
                                ctx.position,
                                parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                seek_flags,
                                UNDEFINED_DIRECTION,
                                global,
                                ctx,
                                tick,
                            );
                        }
                        ProfileRank::Officer => {
                            if !self.alert_soldiers(
                                ctx.position,
                                0,
                                global,
                                grid,
                                ctx,
                                tick,
                                AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier,
                            ) {
                                self.seek_area(
                                    sim,
                                    ctx.position,
                                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                    seek_flags,
                                    UNDEFINED_DIRECTION,
                                    global,
                                    ctx,
                                    tick,
                                );
                            }
                        }
                    }
                }
                return false;
            }

            // If the body is tied or unconscious, say
            // AwakensSleeper, transition to
            // `SeekingBodyAwakeningSleeper`, and launch
            // `WakeUp` with the body as antagonist.  The
            // entity-view map lets us check posture / substate
            // without a second borrow on the entity store.
            let is_tied = view.posture == crate::element::Posture::Tied;
            let is_unconscious = view.ai_substate == Substate::SleepingUnconscious;

            if ctx.self_is_rider || view.is_dead {
                // Dead-body / rider-confirms-dead branch: remember
                // this body so a later officer "examine here" call
                // can short-circuit (see the `CallYourTalk1` arm).
                if !self.already_seen_bodies.contains(&body_handle.get()) {
                    self.already_seen_bodies.push(body_handle.get());
                }
                self.set_state(AiState::Seeking, Substate::SeekingBodyLookingDeadBody);
                self.base.set_emoticon(EmoticonType::XMark);
                self.base.launch_timer(
                    parameters_ai::AI_WATCH_DEADBODY_AGAIN_TIME as u32,
                    ctx.frame,
                );
            } else if is_tied || is_unconscious {
                use crate::element::Command;
                use crate::sequence::{Sequence, SequenceElement};
                self.base.say(Remark::AwakensSleeperr);
                self.set_state(AiState::Seeking, Substate::SeekingBodyAwakeningSleeperr);
                self.base.stop_all();
                let owner = self.base.owner_entity_id;
                let antagonist = Some(ctx.entity_id(body_handle).unwrap_or_else(|| {
                    panic!("SeekingBody wake-up target {body_handle} has no typed live entity view")
                }));
                let mut seq = Sequence::new();
                seq.append_element(SequenceElement::new_interaction(
                    1,
                    Command::WakeUp,
                    owner,
                    antagonist,
                ));
                self.base.outbox.actor.launch_sequences.push(seq);
                self.base.launch_timer(50, ctx.frame);
                self.base.clear_emoticon();
            } else {
                // The body recovered before we arrived. Original
                // abandons the examination immediately even when the
                // timer's preceding IsDetecting check could not see
                // the now-conscious actor.
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
    }

    fn seeking_body_looking_dead_body(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if ctx.self_is_rider {
                self.seek_area(
                    sim,
                    ctx.position,
                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::BODY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            } else {
                if !self.seen_dead_body {
                    self.seen_dead_body = true;
                    self.base.say(Remark::BahIlBougePus);
                }
                if self.examine_other_bodies(ctx, tick) {
                    self.base
                        .my_reconnaissance_report
                        .update(ReportType::DeadBody, ctx.position);
                } else {
                    self.dead_body_alert(
                        sim,
                        ctx.position,
                        SeekFlags::empty(),
                        global,
                        grid,
                        ctx,
                        tick,
                    );
                }
            }
        }
        false
    }

    fn seeking_body_awakening_sleeperr(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // After waking a sleeper (or attempting to) the timer
        // fires; if any other bodies are still pending,
        // `examine_other_bodies` drives off to them;
        // otherwise the report-type governs whether we
        // escalate to a dead-body alert or simply return to
        // duty.
        if stimulus_type == StimulusType::EventTimer && !self.examine_other_bodies(ctx, tick) {
            if self.base.my_reconnaissance_report.report_type == ReportType::DeadBody {
                let pos = self.base.my_reconnaissance_report.seek_position;
                self.dead_body_alert(sim, pos, SeekFlags::empty(), global, grid, ctx, tick);
            } else {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
    }

    // Arrow reactiontime: Say(Arrow), transition to
    // SeekingArrow, run to noise, broadcast HeyFolksLookThere,
    // launch 200-tick timer.

    fn seeking_arrow_reactiontime(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.base.say(Remark::Arrow);
            // Original `SUBSTATE_SEEKING_ARROW_REACTIONTIME` uses plain
            // `GoTo(mposSeekPosition, GOTO_RUN)`.  The nearby seek-point
            // adjustment already happened when the arrow stimulus arrived;
            // this movement must not add GoNear's stop-distance tolerance.
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

    // At noise origin: SeekArea around current position.
    // SOLDIER also sets LOOK_FOR_HELP_AFTER_SEEKING; OFFICER
    // does not.

    fn seeking_arrow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint
            || stimulus_type == StimulusType::EventTimer
        {
            let mut flags = SeekFlags::LOCATION_FIRST | SeekFlags::WALKING;
            if self.get_rank() == ProfileRank::Soldier {
                flags |= SeekFlags::LOOK_FOR_HELP_AFTER;
            }
            let here = ctx.position;
            self.seek_area(sim, here, 0, flags, UNDEFINED_DIRECTION, global, ctx, tick);
        }
        false
    }

    // Arrow just-watching:
    // EVENT_TIMER → Say(Arrow, MYTALK_1) (officer-only
    // soliloquy; the Say wrapper triggers MyTalk1 callback).
    // EVENT_MYTALK_1 → AlertSoldiers (officers only); if no
    // soldier reachable, ReturnToDuty.

    fn seeking_arrow_just_watching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
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
                    global,
                    grid,
                    ctx,
                    tick,
                    AlertSoldiersFailureContinuation::ReturnToDuty,
                ) {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            // Original does not reevaluate combat here. The officer's
            // hint target becomes the center of a plain lost-enemy
            // search as soon as the soldier reaches it.
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
        }
        false
    }

    fn seeking_got_stop_event(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Original adopts the authored alert path before leaving the
            // stopped-seeking state. This explicit gate is required because
            // the generic SetState alert-path switch only covers departures
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
            self.set_state(AiState::Wondering, Substate::WonderingLooking1);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    // ============ CIVILIAN-ALERTS-SOLDIER ================
    // A civilian has run up to this soldier with a CALL_ALERT;
    // these substates walk the soldier through the "listen
    // to civilian → act on report" flow.

    fn seeking_wait_for_alerting_civilian(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::CallReport => {
                // `get_report_from_civilian`: pull the
                // civilian's `ReconnaissanceReport`, merge
                // bodies/charly/type into ours (with the
                // standard delete/add detectable side
                // effects via `consider_report_merged`), then
                // either transition to the alerting state or
                // fall through to the non-alerting "listen
                // and return-to-duty" timer.
                if let StimulusInfo::Hint(ref hint) = stimulus.info {
                    let my_old_report_type = self.base.my_reconnaissance_report.report_type;
                    // Read the civilian's report off their
                    // entity-view snapshot — we can't borrow the
                    // brain mid-think.
                    let civ_view = ctx.entity_view(hint.who_tells_me).unwrap_or_else(|| {
                        panic!(
                            "soldier {} receiving CALL_REPORT requires civilian {} view",
                            self.base.me, hint.who_tells_me
                        )
                    });
                    let civ_report = crate::ai::ReconnaissanceReport {
                        report_type: civ_view.report_type,
                        seek_position: civ_view.report_seek_position,
                        seen_bodies: civ_view.report_seen_bodies.clone(),
                        charly: civ_view.report_charly,
                        charly_seen: civ_view.report_charly.is_some(),
                    };
                    // Merge with all three flags.
                    self.base.consider_report_merged_at_frame(
                        &civ_report,
                        1 | 2 | 4,
                        ctx.entity_views.as_ref(),
                        ctx.frame,
                    );

                    // Alerting transition when the civilian's
                    // report strictly out-ranks ours and is
                    // at least Body.
                    let alerting = civ_report.report_type > my_old_report_type
                        && civ_report.report_type >= ReportType::Body;
                    if alerting {
                        // Original GetReportFromCivilian performs
                        // SetState before Face/LaunchTimer. SetState
                        // cancels the previous substate's timer, so
                        // launching first would silently discard this
                        // talk deadline.
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingGetAlertingReportFromCivilian,
                        );
                        self.base.antagonist = Some(hint.who_tells_me);
                        self.base.face_entity(hint.who_tells_me, ctx);
                        self.base.seek_position = civ_report.seek_position;
                        self.base
                            .my_reconnaissance_report
                            .update(civ_report.report_type, civ_report.seek_position);
                    } else {
                        // Non-alerting branch — wait out the
                        // talk timer in
                        // `SeekingGetReportFromCivilian` then
                        // `ReturnToDuty`.
                        self.set_state(AiState::Seeking, Substate::SeekingGetReportFromCivilian);
                        self.base.face_entity(self.base.antagonist, ctx);
                    }
                    self.base
                        .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_get_report_from_civilian(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Non-alerting civilian report — wait out the
        // talk time and return to duty.
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
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
            self.set_state(
                AiState::Seeking,
                Substate::SeekingGetAlertingReportFromCivilianLook,
            );
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    fn seeking_get_alerting_report_from_civilian_look(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Act on the civilian's report based on rank.
        if stimulus_type == StimulusType::EventTimer {
            let seek_pos = self.base.seek_position;
            match self.get_rank() {
                ProfileRank::Officer => {
                    if self.answer_question(Question::ShallISeekBeforeAlertingSoldiers, ctx) {
                        self.seek_area(
                            sim,
                            seek_pos,
                            0,
                            SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
                    } else if !self.alert_soldiers(
                        seek_pos,
                        0,
                        global,
                        grid,
                        ctx,
                        tick,
                        AlertSoldiersFailureContinuation::ReturnToDuty,
                    ) {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
                ProfileRank::Soldier => {
                    if self.answer_question(Question::ShallISeekBeforeAlertingOfficer, ctx) {
                        self.seek_area(
                            sim,
                            seek_pos,
                            parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                            SeekFlags::LOCATION_FIRST | SeekFlags::LOOK_FOR_HELP_AFTER,
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
                    } else {
                        let returns_to_instructed_group =
                            self.alert_officer_returns_to_instructed_group(tick);
                        let alerted = self.alert_officer(sim, seek_pos, 0, ctx, tick);
                        if alerted && !returns_to_instructed_group {
                            // Original constructs AlertOfficer's GoNear route
                            // inline, then its returned bool controls this
                            // statement's SeekArea fallback. Close the actor
                            // prefix so the engine can construct that route,
                            // and resume this exact tail before EndThink can
                            // translate failure into an independent event.
                            self.base.outbox.reentrant.owner_work.push(
                                crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                    &mut self.base.outbox.actor,
                                )),
                            );
                            self.base
                                .outbox
                                .reentrant
                                .civilian_report_alert_officer_completion_pending = true;
                            self.base.outbox.reentrant.owner_work.push(
                                crate::ai::AiOwnerWork::ResumeCivilianReportAfterAlertOfficer {
                                    seek_position: seek_pos,
                                },
                            );
                        } else if !alerted {
                            self.seek_area(
                                sim,
                                seek_pos,
                                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                                SeekFlags::LOCATION_FIRST,
                                UNDEFINED_DIRECTION,
                                global,
                                ctx,
                                tick,
                            );
                        }
                    }
                }
                ProfileRank::Knight => {
                    self.seek_area(
                        sim,
                        seek_pos,
                        parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                        global,
                        ctx,
                        tick,
                    );
                }
                _ => {}
            }
        }
        false
    }

    /// Resume the statement after the soldier-report branch's synchronous
    /// `AlertOfficer` call. A failed route is consumed by `AlertOfficer` and
    /// makes the caller seek around the retained civilian report position.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resume_civilian_report_after_alert_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        seek_position: Position,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if !self.base.couldnt_reachpoint {
            return;
        }
        self.base.couldnt_reachpoint = false;
        self.seek_area(
            sim,
            seek_position,
            parameters_ai::AI_HINT_SEEK_RADIUS as u16,
            SeekFlags::LOCATION_FIRST,
            UNDEFINED_DIRECTION,
            global,
            ctx,
            tick,
        );
    }

    // ============ OFFICER-SOLDIER COORDINATION ============

    // -------- Officer gives instructions to individual soldier --------

    fn seeking_officer_call_soldier(&mut self, stimulus_type: StimulusType) -> bool {
        // Officer turned to face soldier, now calls them
        if stimulus_type == StimulusType::EventDone {
            let antagonist = self.required_antagonist("calling an individual soldier");
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

    fn seeking_officer_wait_for_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer waits for soldier to approach
        match stimulus_type {
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("waiting for a called soldier");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingSoldierCalledByOfficer
                        | Substate::SeekingSoldierGoToOfficer,
                    ) => {
                        self.face_npc(self.base.antagonist, ctx);
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            StimulusType::CallCoordinate => {
                // Soldier has arrived and reported
                self.set_state(AiState::Seeking, Substate::SeekingOfficerInstructSoldier);
                self.base.point_to(self.base.alert_soldiers_point, ctx);
                self.base.launch_timer(20, ctx.frame);
            }
            _ => {}
        }
        false
    }

    fn seeking_officer_instruct_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer instructs soldier via dialogue
        match stimulus_type {
            StimulusType::CallYourTalk1 => {
                // Soldier said "What's your order, Sir?"
                self.base
                    .say_with_flags(Remark::OfficerSendsOutSoldier, SpeechFlags::MYTALK_1);
            }
            StimulusType::EventMyTalk1 => {
                let antagonist = self.required_antagonist("instructing a soldier");
                // I said "Soldier! Examine this place!"
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        // Original directly calls
                        // `mpAntagonist->Think(CALL_YOURTALK_1)` and
                        // does not retry the call on the speaker.
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::None,
                    });
            }
            StimulusType::CallYourTalk2 => {
                // Soldier said "Sir, yes, Sir!"
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingOfficerWaitForInstructedSoldier,
                );
                self.missed_soldier_timer = 0;
                self.base.launch_timer(30, ctx.frame);
            }
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("waiting for an instructed soldier");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingSoldierGetInstructedByOfficer) {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_officer_wait_for_instructed_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Officer waits for soldier to return from search
        match stimulus_type {
            StimulusType::CallYourTalk1 => {
                self.base.say(Remark::OfficerAsksWhatsup);
            }
            StimulusType::EventTimer => {
                let antagonist =
                    self.required_antagonist("waiting for an instructed soldier to return");
                let ant = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get());
                if let Some(ant) = ant {
                    let alive_and_conscious = ctx
                        .entity_view(ant.handle)
                        .is_some_and(|view| !view.is_dead && !view.is_unconscious);
                    let visible = alive_and_conscious
                        && self.is_detecting_180_degrees(ant.handle as HumanHandle, ctx);
                    if visible && ant.ai_state == AiState::Seeking {
                        self.missed_soldier_timer = 0;
                        self.base.launch_timer(30, ctx.frame);
                    } else if visible {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.missed_soldier_timer += 1;
                        if self.missed_soldier_timer > 100 {
                            // Enough waiting — seek ourselves or alert soldiers
                            if !self.alert_soldiers(
                                ctx.position,
                                0,
                                global,
                                grid,
                                ctx,
                                tick,
                                AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier,
                            ) {
                                self.seek_area(
                                    sim,
                                    ctx.position,
                                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                    SeekFlags::LOCATION_FIRST | self.seek_flags,
                                    UNDEFINED_DIRECTION,
                                    global,
                                    ctx,
                                    tick,
                                );
                            }
                        }
                    }
                } else {
                    self.missed_soldier_timer += 1;
                    if self.missed_soldier_timer > 100 {
                        // Enough waiting — seek ourselves or alert soldiers
                        if !self.alert_soldiers(
                            ctx.position,
                            0,
                            global,
                            grid,
                            ctx,
                            tick,
                            AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier,
                        ) {
                            self.seek_area(
                                sim,
                                ctx.position,
                                parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                                SeekFlags::LOCATION_FIRST | self.seek_flags,
                                UNDEFINED_DIRECTION,
                                global,
                                ctx,
                                tick,
                            );
                        }
                    }
                }
            }
            StimulusType::CallReport => {
                let soldier = match stimulus.info {
                    StimulusInfo::Human(h) => h.get(),
                    _ => self
                        .required_antagonist("receiving an instructed soldier report")
                        .get(),
                };
                if !self.get_report_from_soldier(soldier, false, ctx, tick) {
                    // Nothing special discovered
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingOfficerGetReportFromSoldier,
                    );
                    self.face_npc(self.base.antagonist, ctx);
                    self.base.launch_timer(100, ctx.frame);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_officer_get_report_from_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer received report, wrapping up
        match stimulus_type {
            StimulusType::CallYourTalk1 => {
                self.base
                    .say_with_flags(Remark::OfficerEndsConversation, SpeechFlags::MYTALK_1);
            }
            StimulusType::EventTimer | StimulusType::EventMyTalk1 => {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            _ => {}
        }
        false
    }

    // -------- Soldier receives instructions from officer --------

    fn seeking_soldier_called_by_officer(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier called by officer, approach on timer
        if stimulus_type == StimulusType::EventTimer {
            let antagonist = self.required_antagonist("approaching a calling officer");
            let officer_pos = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == antagonist.get())
                .map(|cs| cs.position)
                .unwrap_or_else(|| {
                    panic!(
                        "called soldier {} requires officer {} in the camp snapshot",
                        self.base.me, antagonist
                    )
                });
            self.go_near(
                AiState::Seeking,
                Substate::SeekingSoldierGoToOfficer,
                officer_pos,
                40,
                // Original's CALLED_BY_OFFICER timer calls
                // `GoNear(Position(mpAntagonist), 40)` without
                // `GOTO_RUN`; the separate return-to-officer path is
                // the one that explicitly runs.
                GotoFlags::empty(),
                ctx,
            );
            self.base.launch_timer(20, ctx.frame);
        }
        false
    }

    fn seeking_soldier_go_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier walking to officer
        match stimulus_type {
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("walking to a calling officer");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::EventReachPoint => {
                let antagonist = self.required_antagonist("reaching a calling officer");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallCoordinate,
                            info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                        },
                    );
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingSoldierGetInstructedByOfficer,
                    );
                    self.base.launch_timer(20, ctx.frame);
                    self.base
                        .say_with_flags(Remark::AwaitsOrders, SpeechFlags::MYTALK_1);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_soldier_get_instructed_by_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier receiving instructions from officer
        match stimulus_type {
            StimulusType::EventMyTalk1 => {
                let antagonist = self.required_antagonist("receiving officer instructions");
                // I said "What's your order, Sir?"
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        // Direct antagonist Think; no sender fallback
                        // in the Original conversation chain.
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::None,
                    });
            }
            StimulusType::CallYourTalk1 => {
                // Officer said "Soldier! Examine this place!"
                // Check if body was already examined
                if self
                    .base
                    .detected_body
                    .is_some_and(|body| self.already_seen_bodies.contains(&body.get()))
                {
                    let antagonist = self.required_antagonist("declining an already examined body");
                    // Already examined — skip search, return to officer
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            // Direct antagonist Think; no sender
                            // fallback in the Original.
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallYourTalk2,
                            info: StimulusInfo::None,
                        },
                    );
                    self.set_state(AiState::Seeking, Substate::SeekingSoldierReturnToOfficer);
                    // Re-dispatch as reachpoint.
                    self.base.launch_timer(1, ctx.frame);
                } else {
                    self.base
                        .say_with_flags(Remark::GiveOrReceiveOrder, SpeechFlags::MYTALK_2);
                }
            }
            StimulusType::EventMyTalk2 => {
                let antagonist = self.required_antagonist("accepting officer instructions");
                // I said "Sir, yes, Sir!"
                // Original captures the officer's selected body before
                // delivering CALL_YOURTALK_2, which can advance the
                // officer's conversation state.
                let officer = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .unwrap_or_else(|| {
                        panic!(
                            "instructed soldier {} requires missing officer antagonist {}",
                            self.base.me, antagonist
                        )
                    });
                let body_handle = officer.detected_body;
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        // Direct antagonist Think; no sender fallback
                        // in the Original conversation chain.
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk2,
                        info: StimulusInfo::None,
                    });
                // Add the body to our detection list so we don't
                // re-react when detecting it later.
                if let Some(body_handle) = body_handle {
                    self.base.outbox.actor.add_detectables.push((
                        ctx.entity_id(body_handle).unwrap_or_else(|| {
                            panic!("CallYourTalk2 body {body_handle} has no typed live entity view")
                        }),
                        crate::element::DetectableType::Body,
                    ));
                }
                self.current_task_priority = task_priority::SEEKING;
                // Read alert_soldiers_point from officer
                self.base.alert_soldiers_point = officer.alert_soldiers_point;
                self.officers_position = officer.position;
                self.seek_area(
                    sim,
                    self.base.alert_soldiers_point,
                    0,
                    SeekFlags::LOCATION_FIRST | SeekFlags::REPORT_OFFICER_AFTER,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            StimulusType::EventTimer => {
                let antagonist =
                    self.required_antagonist("waiting for officer instruction completion");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingOfficerInstructSoldier) {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_soldier_return_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier returning to officer after search
        match stimulus_type {
            StimulusType::EventTimer => {
                // Original dereferences `mpAntagonist` here. It is not
                // necessarily a soldier: retained saves can carry a civilian
                // antagonist while this soldier is returning to the last
                // known officer position. `camp_soldiers` cannot distinguish
                // that valid participant from a missing entity.
                let ant_substate = ctx
                    .expect_entity_view(
                        self.base.antagonist,
                        "soldier-return-to-officer antagonist",
                    )
                    .ai_substate;
                match ant_substate {
                    Substate::SeekingOfficerWaitForInstructedSoldier
                    | Substate::SeekingOfficerWaitForInstructedGroup
                    | Substate::SeekingDetectedCharly => {
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {
                        // Are we near the officer's last known position?
                        let dx = ctx.position.x - self.officers_position.x;
                        let dy = ctx.position.y - self.officers_position.y;
                        let sq_dist = dx * dx + dy * dy;
                        if sq_dist < ctx.sq_standard_view_radius {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                        } else {
                            // Not near enough to know officer left
                            self.base.launch_timer(20, ctx.frame);
                        }
                    }
                }
            }
            StimulusType::EventReachPoint => {
                let ant_substate = ctx
                    .expect_entity_view(
                        self.base.antagonist,
                        "soldier-return-to-officer antagonist",
                    )
                    .ai_substate;
                match ant_substate {
                    Substate::SeekingOfficerWaitForInstructedSoldier
                    | Substate::SeekingOfficerWaitForInstructedGroup => {
                        let antagonist =
                            self.required_antagonist("starting a report to an officer");
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::BeginSoldierGiveReport {
                                officer: antagonist.get(),
                                current_frame: ctx.frame,
                            },
                        );
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_soldier_give_report_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier gives report to officer
        match stimulus_type {
            StimulusType::EventMyTalk1 => {
                let antagonist = self.required_antagonist("giving a report to an officer");
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        // Original directly calls
                        // `mpAntagonist->Think(CALL_YOURTALK_1, mpMe)`;
                        // it never retries on the reporting soldier.
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    });
                self.base.launch_timer(20, ctx.frame);
            }
            StimulusType::EventTimer => {
                self.seek_flags = SeekFlags::empty();
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            _ => {}
        }
        false
    }

    // -------- Officer calls a group --------

    fn seeking_officer_call_group(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer
            && !self.alert_soldiers(
                self.base.seek_position,
                self.seek_flags.bits(),
                global,
                grid,
                ctx,
                tick,
                AlertSoldiersFailureContinuation::ReturnToDuty,
            )
        {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    fn seeking_officer_wait_for_group(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer waits for group to assemble
        if matches!(
            stimulus_type,
            StimulusType::CallCoordinate | StimulusType::EventTimer
        ) {
            // Check if anyone is still approaching
            let mut wait_longer = false;
            self.alerted_us.retain(|&handle| {
                let substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == handle)
                    .map(|cs| cs.ai_substate);
                match substate {
                    Some(
                        Substate::SeekingGroupCalledByOfficer | Substate::SeekingGroupGoToOfficer,
                    ) => {
                        wait_longer = true;
                        true // keep in list
                    }
                    Some(Substate::SeekingGroupGetInstructedByOfficer) => {
                        true // arrived, keep
                    }
                    _ => false, // remove from list
                }
            });

            if !wait_longer {
                if !self.alerted_us.is_empty() {
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerInstructGroup);
                    self.base.launch_timer(10, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
        }
        false
    }

    fn seeking_officer_instruct_group(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        // Officer instructs group — point to position
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(
                AiState::Seeking,
                Substate::SeekingOfficerInstructGroupPointing,
            );
            if self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly {
                self.base.say(Remark::OfficerSendsOutGroupForCharly);
            } else {
                self.base.say(Remark::OfficerSendsOutGroup);
            }
            self.base.point_to(self.base.seek_position, ctx);
        }
        false
    }

    fn seeking_officer_instruct_group_pointing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer done pointing, instruct each soldier
        if stimulus_type == StimulusType::EventDone {
            let mut seek_flags = SeekFlags::REPORT_OFFICER_AFTER;
            let seek_pos = self.base.seek_position;
            if (seek_pos.x - ctx.position.x)
                .abs()
                .max((seek_pos.y - ctx.position.y).abs())
                > 100.0
            {
                seek_flags |= SeekFlags::LOCATION_FIRST;
            }
            // Instruct each soldier via direct CALL_INSTRUCTION and
            // prune refusals from the live list before finalising.
            let instructed = self.alerted_us.clone();

            // When the group is chasing a missed friend who walks a
            // patrol path, the officer spreads the group along that
            // path: every soldier gets its own waypoint as the seek
            // point, stepping by `(len - 1) / (soldiers - 1)` and
            // wrapping. Only then does the whole group keep
            // LOCATION_FIRST — otherwise the officer sends everyone to
            // the same reported position and only the first soldier
            // searches the location itself.
            let mut charly_waypoints: Vec<Position> = Vec::new();
            if self.base.my_reconnaissance_report.report_type == ReportType::MissedCharly {
                seek_flags |= SeekFlags::CHARLY_SEEK;
                let charly = self.base.my_reconnaissance_report.charly;
                let charly_path = ctx.entity_view(charly).and_then(|view| {
                    view.has_patrol_path
                        .then_some(view.patrol_hiking_path_index)
                        .flatten()
                });
                if let Some(path_index) = charly_path
                    && !instructed.is_empty()
                    && let Some(path) = ctx.hiking_paths.get(usize::from(path_index))
                    && !path.waypoints.is_empty()
                {
                    seek_flags |= SeekFlags::LOCATION_FIRST;
                    charly_waypoints = path
                        .waypoints
                        .iter()
                        .map(|w| Position {
                            x: w.x as f32,
                            y: w.y as f32,
                            sector: None,
                            level: w.level,
                        })
                        .collect();
                }
            }

            // Waypoint cursor and stride for the charly-path spread.
            let waypoint_step = if charly_waypoints.is_empty() {
                0
            } else if instructed.len() > 1 {
                (charly_waypoints.len() - 1) / (instructed.len() - 1)
            } else {
                0
            };
            let mut waypoint_index = 0usize;

            tracing::trace!(
                target: "robin_engine::ai_enemy::phalanx",
                officer = self.base.me,
                frame = ctx.frame,
                group = ?instructed,
                charly_waypoints = charly_waypoints.len(),
                "officer instructs its alerted group with CallInstruction"
            );
            let mut pending_instructions = Vec::with_capacity(instructed.len());
            for handle in instructed.iter().copied() {
                let soldier_seek_pos = if charly_waypoints.is_empty() {
                    seek_pos
                } else {
                    let pos = charly_waypoints[waypoint_index];
                    waypoint_index = (waypoint_index + waypoint_step) % charly_waypoints.len();
                    pos
                };
                pending_instructions.push((handle, soldier_seek_pos));
            }
            self.pending_group_instruction_candidates = pending_instructions;
            self.pending_group_instruction_seek_flags = seek_flags.bits();
            self.pending_group_instruction_clear_location_after_accept =
                charly_waypoints.is_empty();
            if self.pending_group_instruction_candidates.is_empty() {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            } else {
                // Original calls each soldier synchronously. A refusal deletes
                // that member and retries the same list index, so only the
                // first *accepted* member consumes LOCATION_FIRST.
                self.queue_next_group_instruction();
            }
        }
        false
    }

    fn seeking_officer_wait_for_instructed_group(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Officer waits for group to report back.
        match stimulus_type {
            StimulusType::CallReport => {
                let soldier = match stimulus.info {
                    StimulusInfo::Human(h) => h.get(),
                    _ => self
                        .required_antagonist("receiving an instructed group report")
                        .get(),
                };
                if !self.get_report_from_soldier(soldier, true, ctx, tick) {
                    // Nothing special detected
                    self.face_npc(soldier, ctx);
                    self.base
                        .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                }
            }
            StimulusType::EventTimer => {
                // Check if there are still seeking soldiers
                self.alerted_us.retain(|&handle| {
                    let member = ctx.entity_view(handle).unwrap_or_else(|| {
                        panic!(
                            "officer waiting for instructed group requires alerted soldier {handle} view"
                        )
                    });
                    assert!(
                        member.is_soldier(),
                        "officer alerted_us handle {handle} resolved to non-soldier {:?}",
                        member.kind
                    );
                    let substate = member.ai_substate;
                    matches!(
                        substate,
                        Substate::SeekingSeekpoint
                                | Substate::SeekingSeekpointWatching
                                | Substate::SeekingSeekpointWatchingSidewards
                                | Substate::SeekingSeekpointPassedAmbushPointLeft
                                | Substate::SeekingSeekpointPassedAmbushPointRight
                                | Substate::SeekingSeekpointCheckingAmbushPoint
                                | Substate::SeekingSeekpointApproachingBeggar
                                | Substate::SeekingSeekpointIdentifyingBeggar1
                                | Substate::SeekingSeekpointIdentifyingBeggar2
                                | Substate::SeekingSoldierReturnToOfficer
                                | Substate::SeekingSoldierGiveReportToOfficer
                                | Substate::SeekingRunningToOfficer
                                | Substate::SeekingRunningToOfficerSeen
                                | Substate::SeekingBodyReactiontime
                                | Substate::SeekingBody
                                | Substate::SeekingNet
                                | Substate::SeekingBodyLookingDeadBody
                                | Substate::SeekingBodyAwakeningSleeperr
                                | Substate::SeekingDetectedCharly
                    )
                });

                if self.alerted_us.is_empty() {
                    let report = &self.base.my_reconnaissance_report;
                    if report.report_type == ReportType::MissedCharly
                        && let Some(charly_handle) = report.charly
                    {
                        let charly = ctx.entity_view(charly_handle).unwrap_or_else(|| {
                            panic!(
                                "officer waiting for instructed group requires Charly {} view",
                                charly_handle
                            )
                        });
                        if matches!(
                            charly.ai_substate,
                            Substate::SeekingCharlySentToOfficer
                                | Substate::SeekingCharlyGoToOfficer
                        ) {
                            self.base.launch_timer(30, ctx.frame);
                            return false;
                        }
                    }
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else {
                    self.base.launch_timer(30, ctx.frame);
                }
            }
            _ => {}
        }
        false
    }

    // -------- Officer alerted group inside house --------

    fn seeking_officer_wait_inside_house_to_instruct_group(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(
                AiState::Seeking,
                Substate::SeekingOfficerLeavingHouseToInstructGroup,
            );
            self.base
                .go_to(self.gather_position, GotoFlags::empty(), ctx);
        }
        false
    }

    fn seeking_officer_leaving_house_to_instruct_group(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.base.face_direction(self.gather_direction, ctx);
            }
            StimulusType::EventDone => {
                self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
                self.base.launch_timer(1, ctx.frame);
            }
            _ => {}
        }
        false
    }

    // -------- Group called by officer --------

    fn seeking_group_called_by_officer(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if self.gather_position_instructed {
                // Original calls the raw GoTo first and performs exactly one
                // SetState below.  Using EnemyAi::go_to here adds a hidden
                // same-state SetState before the movement, which publishes a
                // spurious attentive-mode request and changes the later
                // ReachPoint -> Turn sequence ordering.
                self.base.go_to(self.gather_position, GotoFlags::RUN, ctx);
            } else {
                let antagonist = self.required_antagonist("joining a called officer group");
                let officer_pos = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.position)
                    .unwrap_or_else(|| {
                        panic!(
                            "called group member {} requires officer {} in the camp snapshot",
                            self.base.me, antagonist
                        )
                    });
                self.base.go_near(
                    officer_pos,
                    parameters_ai::AI_TALK_DISTANCE,
                    GotoFlags::RUN,
                    ctx,
                );
            }
            self.set_state(AiState::Seeking, Substate::SeekingGroupGoToOfficer);
            self.base.launch_timer(20, ctx.frame);
        }
        false
    }

    fn seeking_group_go_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("travelling to a group officer");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingOfficerWaitForGroup
                        | Substate::SeekingDetectedCharly
                        | Substate::SeekingOfficerWaitInsideHouseToInstructGroup
                        | Substate::SeekingOfficerLeavingHouseToInstructGroup,
                    ) => {
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            StimulusType::EventReachPoint => {
                if self.gather_position_instructed {
                    // Original FaceTo keys its same-direction shortcut on the
                    // actor's live action state, independently of whether the
                    // ReachPoint arrived recursively. In particular, a GoTo
                    // which was already at its destination retains either the
                    // pre-existing Waiting state (shortcut to EventDone) or a
                    // non-waiting state (author a Turn).
                    self.base.face_direction(self.gather_direction, ctx);
                } else {
                    self.face_npc(self.base.antagonist, ctx);
                }
            }
            StimulusType::EventDone => {
                let antagonist = self.required_antagonist("finishing travel to a group officer");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingOfficerWaitForGroup
                        | Substate::SeekingDetectedCharly
                        | Substate::SeekingOfficerWaitInsideHouseToInstructGroup
                        | Substate::SeekingOfficerLeavingHouseToInstructGroup,
                    ) => {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingGroupGetInstructedByOfficer,
                        );
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: antagonist.get(),
                                stimulus_type: StimulusType::CallCoordinate,
                                info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                            },
                        );
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_group_get_instructed_by_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Group member receives seek instruction
        if stimulus_type == StimulusType::CallInstruction
            && let StimulusInfo::Hint(ref hint) = stimulus.info
        {
            self.base.alert_soldiers_point = hint.seek_point;
            self.officers_position = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == hint.who_tells_me.get())
                .map(|cs| cs.position)
                .unwrap_or_else(|| {
                    panic!(
                        "instructed group member {} requires officer {} in the camp snapshot",
                        self.base.me, hint.who_tells_me
                    )
                });
            // Push a 30-frame THIS_GUY-scoped forbid so this
            // NPC doesn't also auto-speak the same line when
            // the group-instruction chain re-enters
            // SeekingSoldierGiveReportToOfficer.  THIS_GUY
            // scope matches on `guy_index` only, so
            // `speech_id=0` is harmless here.
            self.forbid_remark(
                global,
                Remark::TellsOfficerNothing,
                30,
                crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
                0,
                ctx.original_creation_order
                    .expect("ForbidRemark requires the AI owner's Original creation order"),
                ctx.frame,
            );
            self.seek_area(
                sim,
                hint.seek_point,
                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                SeekFlags::from_bits_truncate(hint.seek_flags),
                UNDEFINED_DIRECTION,
                global,
                ctx,
                tick,
            );
            return true;
        }
        false
    }

    // -------- Soldier alerts officer --------

    fn seeking_running_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
                            .delete_detectables
                            .push(crate::element::DetectableType::Friend);
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
                    if !self.alert_officer(sim, self.base.seek_position, 0, ctx, tick) {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_running_to_officer_seen(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Soldier reached officer, starting report
        match stimulus_type {
            StimulusType::EventMyTalk0 => {
                let antagonist = self.required_antagonist("starting an officer report");
                // Forward talk to officer
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                if matches!(
                    ant_substate,
                    Some(
                        Substate::SeekingOfficerWaitForInstructedSoldier
                            | Substate::SeekingOfficerWaitForAlertingSoldier
                            | Substate::SeekingDetectedCharly
                    )
                ) {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target: antagonist.get(),
                            stimulus_type: StimulusType::CallYourTalk0,
                            info: StimulusInfo::None,
                        },
                    );
                }
            }
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("waiting during an officer report");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingOfficerWaitForInstructedSoldier
                        | Substate::SeekingOfficerWaitForAlertingSoldier
                        | Substate::SeekingDetectedCharly,
                    ) => {
                        self.base.launch_timer(20, ctx.frame);
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            StimulusType::EventReachPoint => {
                let antagonist = self.required_antagonist("reaching an officer to report");
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == antagonist.get())
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingOfficerWaitForInstructedSoldier
                        | Substate::SeekingOfficerWaitForAlertingSoldier
                        | Substate::SeekingDetectedCharly,
                    ) => {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingSoldierGiveAlertingReportToOfficerStart,
                        );
                        // Say remark based on report type
                        let speech = SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY;
                        match self.base.my_reconnaissance_report.report_type {
                            ReportType::Body | ReportType::DeadBody => {
                                self.base.say_with_flags(Remark::TellsOfficerBody, speech);
                            }
                            ReportType::Enemy => {
                                self.base.say_with_flags(Remark::TellsOfficerEnemy, speech);
                            }
                            ReportType::MissedCharly => {
                                self.base
                                    .say_with_flags(Remark::TellsOfficerCharlyAway, speech);
                            }
                            _ => {
                                self.base.say_with_flags(Remark::TellsOfficerOther, speech);
                            }
                        }
                        // `RHartificialmalignity.cpp:3350` runs this
                        // `LaunchTimer(150)` *after* the `Say(...,
                        // SPEECH_MYTALK_1 | SPEECH_EMERGENCY)` above has
                        // fully returned. When that Say is rejected -- the
                        // common case here, because the reporting soldier is
                        // still blipped (`RHartificialintelligence.cpp:5924`)
                        // -- `InformAIOnFinishedRemark`
                        // (`RHartificialintelligence.cpp:6264`) re-enters
                        // `Think(EVENT_MYTALK_1)` on this same call stack.
                        // That nested Think walks the report-start substate
                        // into ..._POINT and launches its own 100-frame timer
                        // (`RHartificialmalignity.cpp:3398`), and only then
                        // does this statement overwrite it with 150. Rust
                        // settles speech at the owner return boundary, so the
                        // timer has to ride the same owner-work FIFO to land
                        // behind the nested Think instead of ahead of it.
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::LaunchTimer {
                                frames: 150,
                                current_frame: ctx.frame,
                            },
                        );
                    }
                    _ => {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_soldier_give_alerting_report_to_officer_start(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Start of alerting report to officer.
        // Compare our report type vs officer's to decide whether
        // to point direction (new info) or just end (redundant).
        if matches!(
            stimulus_type,
            StimulusType::EventMyTalk1 | StimulusType::EventTimer
        ) {
            let antagonist = self.required_antagonist("giving an alerting report");
            let officer_report = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == antagonist.get())
                .map(|cs| cs.report_type)
                .unwrap_or_else(|| {
                    panic!(
                        "reporting soldier {} requires officer {} in the camp snapshot",
                        self.base.me, antagonist
                    )
                });

            let point_direction = match self.base.my_reconnaissance_report.report_type {
                ReportType::Nothing => false,
                ReportType::Noise => officer_report == ReportType::Nothing,
                ReportType::Body | ReportType::DeadBody => officer_report <= ReportType::Noise,
                ReportType::Enemy => officer_report <= ReportType::DeadBody,
                ReportType::MissedCharly => officer_report == ReportType::Nothing,
            };

            if point_direction {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallReport,
                        info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    });
                // Original invokes both recipient Think calls before
                // changing the caller's substate. The officer's
                // CALL_YOURTALK_1 handling can synchronously call this
                // soldier back, and that callback must still observe
                // the report-start substate.
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RequestThinkResult {
                        target: antagonist.get(),
                        caller: self.base.me,
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::None,
                        continuation: ThinkResultContinuation::SoldierFinishedAlertReportStart,
                    },
                );
            } else {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                );
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallReport,
                        info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    });
                self.base
                    .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
            }
        }
        false
    }

    fn seeking_soldier_give_alerting_report_to_officer_point(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        // Soldier points to location
        match stimulus_type {
            StimulusType::CallYourTalk1 | StimulusType::EventTimer => {
                self.base.say(Remark::TellsOfficerWhere);
                self.base.point_to(self.base.seek_position, ctx);
            }
            StimulusType::EventDone => {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
                );
                self.face_npc(self.base.antagonist, ctx);
                self.base
                    .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
            }
            _ => {}
        }
        false
    }

    fn seeking_soldier_give_alerting_report_to_officer_end(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // End of alerting report
        if stimulus_type == StimulusType::EventTimer {
            self.seek_flags = SeekFlags::empty();
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // -------- Officer is alerted by soldier --------

    fn seeking_officer_wait_for_alerting_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::CallYourTalk0 => {
                self.base.say(Remark::OfficerAsksWhatsup);
            }
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("waiting for an alerting soldier");
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
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
                }
            }
            StimulusType::CallReport => {
                let soldier = match stimulus.info {
                    StimulusInfo::Human(h) => h.get(),
                    _ => self
                        .required_antagonist("receiving an alerting soldier report")
                        .get(),
                };
                if !self.get_report_from_soldier(soldier, false, ctx, tick) {
                    // Nothing really alerting
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingOfficerGetReportFromSoldier,
                    );
                    self.face_npc(self.base.antagonist, ctx);
                    self.base
                        .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
                }
            }
            _ => {}
        }
        false
    }

    fn seeking_officer_get_alerting_report_from_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Officer processes alerting report
        match stimulus_type {
            StimulusType::CallYourTalk1 => {
                self.base.say_with_flags(
                    Remark::OfficerAsksWhere,
                    SpeechFlags::MYTALK_1 | SpeechFlags::EMERGENCY,
                );
            }
            StimulusType::EventMyTalk1 => {
                let antagonist = self.required_antagonist("answering an alerting soldier");
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: antagonist.get(),
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::None,
                    });
            }
            StimulusType::EventTimer
                if !self.alert_soldiers(
                    self.base.seek_position,
                    0,
                    global,
                    grid,
                    ctx,
                    tick,
                    AlertSoldiersFailureContinuation::ReturnToDuty,
                ) =>
            {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            _ => {}
        }
        false
    }

    // ============ ATTACKING ============

    fn seeking_knight_watching_tower_guard(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Knight reacts directly on alerts:
            //   SeekArea(seek_position, AI_HINT_SEEK_RADIUS, LOCATION_FIRST);
            self.seek_area(
                sim,
                self.base.seek_position,
                parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
                UNDEFINED_DIRECTION,
                global,
                ctx,
                tick,
            );
        }
        false
    }

    // Freeing someone from the net: wait out, or reach point
    // and take the net.

    fn seeking_net(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // If detected body is no longer stuck under net
                // AND I'm detecting them → resurrected,
                // ReturnToDuty; else re-arm timer.  This is the
                // cone-and-LOS `IsDetecting`, not the 360° feel
                // bubble, and it is short-circuited behind the net
                // check so a still-trapped body costs no LOS query.
                let body_stuck = ctx
                    .expect_entity_view(self.base.detected_body, "seeking-net body")
                    .stuck_under_net;
                if !body_stuck && self.is_detecting(self.base.detected_body, ctx) {
                    // Resurrected.
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else {
                    self.base.launch_timer(10, ctx.frame);
                }
            }
            StimulusType::EventReachPoint => {
                // If detected body is still under net, riders just
                // SeekArea around themselves; foot units launch
                // the SEARCH×4+TAKE sequence + transition to
                // SeekingTakingNet.  Otherwise ReturnToDuty.
                let body_stuck = ctx
                    .expect_entity_view(self.base.detected_body, "seeking-net body")
                    .stuck_under_net;
                if body_stuck {
                    if ctx.self_is_rider {
                        // Rider can't dismount to take the net;
                        // expand the seek radius and look.
                        let here = ctx.position;
                        self.seek_area(
                            sim,
                            here,
                            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                            SeekFlags::BODY_SEEK,
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    // Finished removing a net: free another, examine body,
    // or return to duty.

    fn seeking_taking_net(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // 3-way branch on detected body state:
            //   stuck-under-net → RunToFreeNetVictim (still
            //     another net on top)
            //   dead|unconscious → RunToExamineBody
            //   else → ReturnToDuty
            let body = self.base.detected_body.unwrap_or_else(|| {
                panic!(
                    "enemy AI {} taking a net requires a detected body",
                    self.base.me
                )
            });
            let view = ctx.expect_entity_view(body, "taking-net body");
            let stuck = view.stuck_under_net;
            let examine = !view.is_able_to_fight && !view.stuck_under_net;
            if stuck || examine {
                // `run_to_examine_body` internally forks on
                // `stuck_under_net` and transitions into
                // SeekingNet for the net-takedown path, or
                // SeekingBody for the examine path.
                self.run_to_examine_body(body.get(), ctx, tick, grid);
            } else {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }
        false
    }

    // Officer scanning for free soldiers (three stages).

    fn seeking_officer_looking_for_soldiers1(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
            // FaceTo((direction + 5) % 16)
            let new_dir = (ctx.direction + 5) % 16;
            self.base.face_direction(new_dir, ctx);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    // Stage-3 sidewards complete; done looking.

    fn seeking_officer_looking_for_soldiers3_sidewards(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // Charly search path: step through `search_charly_way`
    // on each reach point.

    fn seeking_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            if !self.search_charly_way.is_empty() {
                self.search_charly_way.remove(0);
            }
            if self.search_charly_way.is_empty() {
                // If checkpoint_charly == 0 → ReturnToDuty;
                // else transition to CharlyWatching +
                // LookSidewards(LeftRight).
                if self.base.checkpoint_charly.is_none() {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else {
                    self.set_state(AiState::Seeking, Substate::SeekingCharlyWatching);
                    self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
                }
            } else {
                // GoTo next waypoint with RUN (+ DONT_STOP
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
        false
    }

    // Done watching at charly checkpoint: trigger
    // missed-charly alert.

    fn seeking_charly_watching(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.missed_charly_alert(sim, global, ctx, tick, grid);
        }
        false
    }

    // Detected charly reaction; rank-dependent follow-up.

    fn seeking_detected_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
                    let previous_state = AiState::try_from(self.previous_state as u32)
                        .unwrap_or_else(|_| {
                            panic!(
                                "live mPreviousState contains invalid Original enum word {}",
                                self.previous_state
                            )
                        });
                    let previous_substate = Substate::try_from(self.previous_substate as u32)
                        .unwrap_or_else(|_| {
                            panic!(
                                "live mPreviousSubstate contains invalid Original enum word {}",
                                self.previous_substate
                            )
                        });
                    self.set_state(previous_state, previous_substate);
                    self.base.launch_timer(10, ctx.frame);
                }
                _ => {
                    // Soldier, Knight, or officer with no alerted
                    // soldiers all fall through to ReturnToDuty.
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
        }
        false
    }

    // Officer sends charly away toward another officer.

    fn seeking_send_charly_to_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventMyTalk1 => {
                let charly = self.base.friend_in_trouble;
                let Some(charly) = charly else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    return false;
                };
                let antagonist = self.required_antagonist("sending Charly to another officer");
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
                // Original `SUBSTATE_SEEKING_SEND_CHARLY_TO_OFFICER` faces
                // `mpFriendInTrouble` between SetState and LaunchTimer when
                // the second speech callback completes.
                self.base.face_entity(self.base.friend_in_trouble, ctx);
                self.base.launch_timer(100, ctx.frame);
            }
            _ => {}
        }
        false
    }

    // Watching a charly who's been sent off; timer returns
    // to duty.

    fn seeking_looking_resurrected_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // Charly was sent to officer; go near the officer.

    fn seeking_charly_sent_to_officer(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
            // GoNear(antagonist.position, 40);
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                let antagonist = self.required_antagonist("reporting back to an officer");
                // Original: mpMe->IsDetecting(mpAntagonist). This is
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            _ => {}
        }
        false
    }

    // Charly reached officer; reach→CallCoordinate; timer
    // keeps polling.

    fn seeking_charly_go_to_officer_seen(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // Only re-arm if antagonist is in
                // `OfficerWaitForCharly`; else ReturnToDuty.
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
        false
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
                StimulusType::EventMyTalk1
                    // antagonist.think(CallYourTalk1)
                    if self.base.antagonist.is_some() => {
                        let antagonist = self.required_antagonist("answering an officer lecture");
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
                _ => {}
            }
        false
    }

    // Officer waits for
    // charly: on timer inspect antagonist substate; on coordinate
    // call, rebuke.

    fn seeking_officer_wait_for_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // If antagonist is still in one of the
                // "on the way" charly substates, face them, clear the
                // emoticon and re-arm the timer; else ReturnToDuty.
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
        false
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
                let antagonist = self.required_antagonist("lecturing Charly");
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
                // the officer's position and PointTo it; else PointTo
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
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
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
        false
    }

    // Civilian reached soldier to report; waits for officer's
    // wait-state.

    fn seeking_civilian_running_to_soldier_seen(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventTimer => {
                // If antagonist is still WaitForAlertingCivilian, re-arm
                // timer; else end.
                let officer_waiting = ctx
                    .expect_entity_view(self.base.antagonist, "civilian-report soldier")
                    .ai_substate
                    == Substate::SeekingWaitForAlertingCivilian;
                if officer_waiting {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::EventReachPoint => {
                // Only transition if antagonist still in
                // wait-for-alerting-civilian; else ReturnToDuty.
                let officer_waiting = ctx
                    .expect_entity_view(self.base.antagonist, "civilian-report soldier")
                    .ai_substate
                    == Substate::SeekingWaitForAlertingCivilian;
                if officer_waiting {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                    );
                    self.base.launch_timer(10, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            _ => {}
        }
        false
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

    /// Handle the body of Original `Think(EVENT_REACHPOINT)` while seeking an
    /// authored seek point. This is also called re-entrantly by the two
    /// passed-ambush substates, matching their direct `Think` call.
    fn reached_seek_point(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
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

            // Original increments the count before `rand() % count`, so even
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
            self.seek_next_point(sim, global, ctx, tick);
        }
    }

    fn think_expected_menacing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        match self.base.current_substate {
            Substate::MenacingPcInComa if stimulus_type == StimulusType::EventTimer => {
                // Assert IsPC + check MaxNormDistance < 100 &&
                // IsUnconscious() && IsInComa(). `is_pc` and `in_coma`
                // both live on `AiEntityView`, so the full triplet is
                // checkable here without approximation.
                let keep_watching = {
                    let v =
                        ctx.expect_entity_view(self.base.primary_target, "menacing-coma target");
                    // `MaxNormDistance`
                    // (`original-code/RHartificialintelligence.cpp:6950-6953`)
                    // is the stretched **3D** Chebyshev distance between
                    // the two `GetPosition()` world points, so a target
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }

            // Reached arrow reserves: refill ammo and SeekArea around.
            _ => {}
        }
        false
    }

    fn think_expected_fleeing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
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
                    // through the virtual Enemy ReturnToDuty override. The
                    // base Rust common handler cannot call back into its
                    // containing EnemyAi, so close that virtual call here.
                    //
                    // original-code/RHartificialintelligence.cpp:1950-1955
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    return true;
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
                        // toward the door, re-issue the GoTo.  Gated
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
                            // Arrived at door PositionIn — now run to PointOut
                            // to exit the map (launches a sequence
                            // element targeting the door's PointOut).
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

            // Merry man has reached PointOut — deactivate.
            Substate::FleeingMerryManLeaveMap => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // `non_script_lock(Freeze); set_active(false);`
                    self.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
                    self.base.outbox.actor.deactivate = true;
                }
            }

            // ============ BULK-PORTED HANDLERS ============
            //
            // The block below ports the ~83 substates that were
            // previously swept into the no-op group when the
            // exhaustive-match refactor landed.  Many of these arms
            // call helpers that were not yet ported when this block first
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
                        sim,
                        self.base.seek_position,
                        parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::LOCATION_FIRST,
                        UNDEFINED_DIRECTION,
                        global,
                        ctx,
                        tick,
                    );
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
                        global,
                        grid,
                        ctx,
                        tick,
                        AlertSoldiersFailureContinuation::FleeingRunToDoor,
                    );
                    if !alerted {
                        // Fire a self-stimulus so the re-delivery happens
                        // on the next think rather than recursing here
                        // (the reference's direct Think() call is a
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

            // Turned: if detecting target, BattleDecisions, else overview.
            Substate::FleeingRetireFromCombatTurn => {
                if stimulus_type == StimulusType::EventDone {
                    if self.base.primary_target.is_some_and(|primary_target| {
                        self.is_detecting_180_degrees(primary_target, ctx)
                    }) {
                        self.battle_decisions(sim, global, ctx, tick, grid);
                    } else {
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
            }

            // Sword fight step back reached point: resume swordfight with
            // 20-tick timer.
            _ => {}
        }
        false
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
