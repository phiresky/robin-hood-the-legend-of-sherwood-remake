//! `EnemyAi::think_expected_event` — the substate state machine.
//!
//! Lifted out of `ai_enemy/mod.rs` to keep the giant per-substate match
//! manageable. Lives in a separate `impl EnemyAi` block; child modules
//! see the parent's private fields and helpers.

use crate::ai::*;
use crate::parameters_ai;

use super::util::{ai_square_distance, resolve_seek_point_id, vec_to_sector};
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
    /// target is null therefore returns `Some(0)`; only the absence of both
    /// soldier neighbours authorizes `GetNewPrimaryTarget`.
    pub(super) fn phalanx_neighbour_primary_target(
        &self,
        tick: &AiPerTickData,
    ) -> Option<HumanHandle> {
        for neighbour in [self.left_combat_neighbour, self.right_combat_neighbour] {
            if neighbour == 0 {
                continue;
            }
            let fighter = self.find_fighter(neighbour, tick).unwrap_or_else(|| {
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
        match self.base.current_substate {
            Substate::SleepingAwakening => {
                if matches!(
                    stimulus_type,
                    StimulusType::EventDone | StimulusType::EventTimer
                ) {
                    if let Some(alert_path_id) = self.base.alert_path_id
                        && !self.changed_to_alert_path
                    {
                        self.changed_to_alert_path = true;
                        // Rebuild the patrol path from the alert-path
                        // hiking path index.
                        let hiking_paths = &ctx.hiking_paths;
                        self.base.patrol_path =
                            crate::ai::PatrolPath::new(alert_path_id, hiking_paths);
                        self.base.has_patrol_path = self.base.patrol_path.is_some();
                    }
                    self.base.set_emoticon(EmoticonType::QuestionMark);
                    self.set_state(AiState::Wondering, Substate::WonderingLooking1);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            // ============ DEFAULT (common) ============
            _ => {}
        }
        false
    }

    fn think_expected_default_event(
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
                        // search_charly(sim, ).
                        self.search_charly(sim, ctx, tick);
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
                self.wondering_approaching_money(stimulus_type, ctx)
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
                self.wondering_brawl_approaching(stimulus_type, ctx)
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
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint
            || stimulus_type == StimulusType::EventTimer
        {
            self.set_state(AiState::Wondering, Substate::WonderingTakingMoney);
            // Launch a single-element interaction sequence
            // (Take, me, interesting_object) to trigger the
            // pick-up animation on the coin.  The engine
            // launches it at post-think time.
            let obj = self.base.interesting_object;
            if obj != 0 {
                use crate::element::Command;
                use crate::sequence::{Sequence, SequenceElement};
                let owner = self.base.owner_entity_id;
                let antagonist = Some(crate::element::EntityId::Bonus(crate::entity_id::BonusId(
                    obj,
                )));
                let mut seq = Sequence::new();
                seq.append_element(SequenceElement::new_interaction(
                    1,
                    Command::Take,
                    owner,
                    antagonist,
                ));
                self.base.outbox.actor.launch_sequences.push(seq);
            }
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    fn wondering_taking_money(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Check for more money
            self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.base.launch_timer(
                parameters_ai::AI_ARE_THERE_MORE_DOLLARS_LOOKS as u32 * 10,
                ctx.frame,
            );
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
                    self.base.detected_body = next as HumanHandle;
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

    // Ale reactiontime: if shall-take-ale and beer is alive:
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
            if self.answer_question(Question::ShallITakeAle, ctx)
                && let Some(obj_pos) = ctx.entity_position(self.base.interesting_object)
            {
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
                    let obj = self.base.interesting_object;
                    if obj != 0 {
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        let owner = self.base.owner_entity_id;
                        let antagonist = Some(crate::element::EntityId::Bonus(
                            crate::entity_id::BonusId(obj),
                        ));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::DrinkAle,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                    self.base.launch_timer(60, ctx.frame);
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
        if stimulus_type == StimulusType::EventTimer {
            self.base.blood_alcohol = self.base.blood_alcohol.saturating_add(30);
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
                    if self.base.antagonist != 0 => {
                        self.base
                            .outbox.reentrant.cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
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
                    //
                    // Shape 1 contract forbids same-substate
                    // re-issue; route through MoneyReactiontime,
                    // whose timer already advances back to
                    // RunningForMoney.
                    if let Some(obj_pos) = ctx.entity_position(self.base.interesting_object) {
                        self.go_near(
                            AiState::Wondering,
                            Substate::WonderingMoneyReactiontime,
                            obj_pos,
                            parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                            crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::FIND_ACCESSIBLE,
                            ctx,
                        );
                        self.base.launch_timer(1, ctx.frame);
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
                                info: crate::ai::StimulusInfo::Human(self.base.me as HumanHandle),
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
                if obj != 0 && close_enough {
                    // StopAll + Take sequence.
                    self.base.stop_all();
                    use crate::element::Command;
                    use crate::sequence::{Sequence, SequenceElement};
                    let owner = self.base.owner_entity_id;
                    let antagonist = Some(crate::element::EntityId::Bonus(
                        crate::entity_id::BonusId(obj),
                    ));
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
                        object: obj as crate::ai::ObjectHandle,
                        thief: self.base.me,
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
        stimulus_type: StimulusType,
        ctx: &AiContext,
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
                // Check friend_in_trouble substate:
                // - if asleep (Sleeping) → skip hit, go to hitting;
                // - if distance > AI_HIT_DISTANCE+3 → re-issue GoNear;
                // - else actually transition to hitting.
                // An already-cleared brawl partner (handle 0)
                // deliberately skips the sleeping shortcut — the
                // original treats a missing partner as an error
                // arm, not as "asleep".
                let friend_sleeping = self.base.friend_in_trouble != 0
                    && ctx
                        .expect_entity_view(
                            self.base.friend_in_trouble,
                            "brawl-approach friend in trouble",
                        )
                        .ai_state
                        == AiState::Sleeping;
                if friend_sleeping {
                    // Drop the sleeping friend from
                    // `money_fight_enemies` so subsequent brawl
                    // arms don't keep re-targeting a KO'd soldier
                    // (and so `wants_to_continue_money_fight`'s
                    // size-based threshold isn't skewed).
                    let fit = self.base.friend_in_trouble as NpcHandle;
                    self.money_fight_enemies.retain(|h| *h != fit);
                    self.base.friend_in_trouble = 0;
                }
                self.set_state(AiState::Wondering, Substate::WonderingBrawlHitting);
            }
            _ => {}
        }
        false
    }

    // Brawl hit resolution; civilians panic, chase chain continues.

    fn wondering_brawl_hitting(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // Scan camp for an officer who might hear/see me
            // and alert them with EventSeesBrawl.
            self.maybe_officer_sees_me_fighting(ctx, tick);
            // Broadcast civilian panic for anyone in view
            // radius. Queued in the owner-work FIFO so surrounding
            // synchronous AI calls retain their statement order.
            self.nearby_civilians_panic();

            // Remove KO'd target from the enemy list.
            if self.base.friend_in_trouble != 0 {
                let fit = self.base.friend_in_trouble as NpcHandle;
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
                let fit_ok = self.base.friend_in_trouble != 0
                    && !ctx
                        .expect_entity_view(self.base.friend_in_trouble, "brawl-hitting friend")
                        .is_unconscious;
                if fit_ok {
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
                    self.base.face_entity(self.base.friend_in_trouble, ctx);
                    self.base.launch_timer(30, ctx.frame);
                } else if let Some(next) = self.get_nearest_money_fight_enemy(ctx) {
                    self.base.friend_in_trouble = next as HumanHandle;
                    self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
                    self.base.launch_timer(10, ctx.frame);
                } else {
                    // stop_brawling_and_collect_money().
                    self.stop_brawling_and_collect_money(ctx, tick);
                }
            }
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
            // SetEmoticon(Thunderstorm).
            self.base.set_emoticon(EmoticonType::Thunderstorm);
            // Insert friend_in_trouble into money_fight_enemies
            // (asserts soldier + non-self).
            let fit = self.base.friend_in_trouble;
            if fit != 0 && fit != self.base.me {
                let is_soldier = ctx
                    .expect_entity_view(fit, "brawl-got-hit attacker")
                    .is_soldier();
                if is_soldier && !self.money_fight_enemies.contains(&(fit as NpcHandle)) {
                    self.money_fight_enemies.push(fit as NpcHandle);
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
                self.base.friend_in_trouble = next as HumanHandle;
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
            let body = self.base.detected_body;
            // A cleared body handle takes the too-far arm below;
            // a live handle must resolve to a view.
            let (body_pos, is_tied) = if body == 0 {
                (Position::default(), false)
            } else {
                let v = ctx.expect_entity_view(body, "loot-approach body");
                (v.position, v.posture == crate::element::Posture::Tied)
            };
            let dx = body_pos.x - ctx.position.x;
            let dy = body_pos.y - ctx.position.y;
            let dist = dx.abs().max(dy.abs());
            if body == 0 || dist > 100.0 {
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
                self.base.my_reconnaissance_report.add_seen_body(body);
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
                    .unwrap_or(0);
                self.set_state(AiState::Wondering, Substate::WonderingLooting);
                self.base.stop_all();
                let owner = self.base.owner_entity_id;
                let antagonist = Some(crate::element::EntityId::Soldier(
                    crate::entity_id::SoldierId(body),
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
                .unwrap_or(0);
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
                self.base.detected_body = next as HumanHandle;
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
                self.base.object_of_desire = next;
                self.base.interesting_object = next;
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
                    .filter(|h| *h != antagonist && *h != self.base.me)
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
                if antagonist != 0 {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: antagonist,
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
                if self.base.antagonist != 0 {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: self.base.antagonist,
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
            let body = self.base.detected_body;
            if body != 0 {
                let antagonist = Some(crate::element::EntityId::Soldier(
                    crate::entity_id::SoldierId(body),
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
                self.seeking_seekpoint_approaching_beggar(stimulus_type, ctx)
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
                return self
                    .seeking_soldier_give_alerting_report_to_officer_point(stimulus_type, ctx);
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
                return self
                    .seeking_civilian_give_alerting_report_to_soldier_start(stimulus_type, ctx);
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint => {
                return self
                    .seeking_civilian_give_alerting_report_to_soldier_point(stimulus_type, ctx);
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
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        // Soldier is walking toward the beggar's last known
        // position (set by seek_next_point → go_near).
        // On arrival, stop and begin identification.
        if stimulus_type == StimulusType::EventReachPoint {
            // The reference checks MaxNormDistance(beggar) < 100;
            // since we used go_near(sim, pos, 50), reaching means
            // we're close enough. If we somehow aren't, resume.
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
                .entity_view(self.beggar_to_examine)
                .unwrap_or_else(|| {
                    panic!(
                        "beggar {} disappeared before identification turn",
                        self.beggar_to_examine
                    )
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
                let timer = if self.beggar_is_npc { 50 } else { 100 };
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
            if self.beggar_is_npc {
                // Real beggar: NPC shows face and identifies
                // themselves. Transition to phase 2 (wait,
                // then resume seeking).
                // Launch a `BeggarShowFace` sequence element on
                // the beggar via `pending_launch_on_target`,
                // which carries (target, cmd) to the
                // engine-side sequence-manager drain.
                self.base.outbox.actor.launch_on_target.push((
                    self.beggar_to_examine,
                    crate::element::Command::BeggarShowFace,
                ));
                // Original immediately follows the show-face launch
                // with `beggar->Say(CIV_REMARK_BEGGAR_IDENTIFIES_HIMSELF)`.
                // Keep both calls in the ordered actor-effect prefix:
                // SetState below snapshots that prefix before its
                // synchronous script callback.
                self.base.outbox.actor.say_on_target.push((
                    self.beggar_to_examine,
                    crate::ai::Remark::CivBeggarIdentifiesHimself,
                ));
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointIdentifyingBeggar2,
                );
                self.base.launch_timer(50, ctx.frame);
            } else {
                // Disguised PC detected! Set as primary target
                // and begin combat.
                self.base.primary_target = self.beggar_to_examine;
                self.list_them.clear();
                self.list_them.push(self.beggar_to_examine);

                if self.is_archer() {
                    // False beggar stands up via `LeaveBeggar`,
                    // then the archer transitions to
                    // AttackingBowShooting and shoots.
                    self.base
                        .outbox
                        .actor
                        .launch_on_target
                        .push((self.beggar_to_examine, crate::element::Command::LeaveBeggar));
                    self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                    self.shoot_arrow_at(self.base.primary_target, ctx, tick);
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
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            let do_not_investigate = match self.get_rank() {
                ProfileRank::Officer => {
                    let me = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(
                        self.base.me,
                    ));
                    let has_patrol = tick
                        .camp_soldiers
                        .iter()
                        .any(|cs| cs.patrol_chief == Some(me));
                    let dx = (ctx.position.x - self.base.seek_position.x).abs();
                    let dy = (ctx.position.y - self.base.seek_position.y).abs();
                    const OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE: f32 = 100.0;
                    has_patrol || dx.max(dy) > OFFICER_EXAMINE_NOISE_HIMSELF_DISTANCE
                }
                // Soldier / knight: defer to ShallIFollowSteps.
                _ => !self.answer_question(Question::ShallIFollowSteps, ctx),
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
            let body = self.base.detected_body;
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
                        if cs.detectable_bodies.contains(&body) {
                            return None;
                        }
                        Some(cs.handle)
                    });
                }
                ProfileRank::Officer => {
                    // If body is further than
                    // OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE,
                    // delegate via ShallISendOutSoldier.
                    let dist = ctx
                        .entity_view(body)
                        .map(|v| {
                            let dx = (v.position.x - ctx.position.x).abs();
                            let dy = (v.position.y - ctx.position.y).abs();
                            dx.max(dy)
                        })
                        .unwrap_or(0.0);
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
                self.run_to_examine_body(body, ctx, tick, grid);
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
            let body_handle = self.base.detected_body;
            let view = ctx.entity_view(body_handle).unwrap_or_else(|| {
                panic!("SeekingBody timer target {body_handle} has no typed live entity view")
            });
            if !view.is_dead && !view.is_unconscious && self.is_detecting(body_handle, ctx) {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            } else {
                self.base.launch_timer(10, ctx.frame);
            }
        } else if stimulus_type == StimulusType::EventReachPoint {
            let body_handle = self.base.detected_body;
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
                if body_handle != 0 && !self.already_seen_bodies.contains(&body_handle) {
                    self.already_seen_bodies.push(body_handle);
                }
                self.set_state(AiState::Seeking, Substate::SeekingBodyLookingDeadBody);
                self.base.set_emoticon(EmoticonType::XMark);
                self.base.launch_timer(
                    parameters_ai::AI_WATCH_DEADBODY_AGAIN_TIME as u32,
                    ctx.frame,
                );
            } else if body_handle != 0 && (is_tied || is_unconscious) {
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
            self.go_near(
                AiState::Seeking,
                Substate::SeekingArrow,
                self.base.seek_position,
                parameters_ai::AI_MIN_SEARCHNOISE_DISTANCE,
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
                        charly_seen: civ_view.report_charly != 0,
                    };
                    // Merge with all three flags.
                    self.base.consider_report_merged(
                        &civ_report,
                        1 | 2 | 4,
                        ctx.entity_views.as_ref(),
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
                        self.base.antagonist = hint.who_tells_me;
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
                    } else if !self.alert_officer(sim, seek_pos, 0, ctx, tick) {
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

    // ============ OFFICER-SOLDIER COORDINATION ============

    // -------- Officer gives instructions to individual soldier --------

    fn seeking_officer_call_soldier(&mut self, stimulus_type: StimulusType) -> bool {
        // Officer turned to face soldier, now calls them
        if stimulus_type == StimulusType::EventDone {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::RequestThinkResult {
                    target: self.base.antagonist,
                    caller: self.base.me,
                    stimulus_type: StimulusType::CallHey,
                    info: StimulusInfo::Human(self.base.me),
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                        target: self.base.antagonist,
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                let ant = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist);
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
                    StimulusInfo::Human(h) => h,
                    _ => self.base.antagonist,
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
            let officer_pos = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == self.base.antagonist)
                .map(|cs| cs.position)
                .unwrap_or(self.officers_position);
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::EventReachPoint => {
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .map(|cs| cs.ai_substate);
                if ant_substate == Some(Substate::SeekingOfficerWaitForSoldier) {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallCoordinate,
                            info: StimulusInfo::Human(self.base.me),
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
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::None,
                    });
            }
            StimulusType::CallYourTalk1 => {
                // Officer said "Soldier! Examine this place!"
                // Check if body was already examined
                if self.base.detected_body != 0
                    && self.already_seen_bodies.contains(&self.base.detected_body)
                {
                    // Already examined — skip search, return to officer
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            // Direct antagonist Think; no sender
                            // fallback in the Original.
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target: self.base.antagonist,
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
                // I said "Sir, yes, Sir!"
                // Original captures the officer's selected body before
                // delivering CALL_YOURTALK_2, which can advance the
                // officer's conversation state.
                let officer = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .unwrap_or_else(|| {
                        panic!(
                            "instructed soldier {} requires missing officer antagonist {}",
                            self.base.me, self.base.antagonist
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
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallYourTalk2,
                        info: StimulusInfo::None,
                    });
                // Add the body to our detection list so we don't
                // re-react when detecting it later.
                if body_handle != 0 {
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                let ant = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist);
                if let Some(ant) = ant {
                    match ant.ai_substate {
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
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::EventReachPoint => {
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .map(|cs| cs.ai_substate);
                match ant_substate {
                    Some(
                        Substate::SeekingOfficerWaitForInstructedSoldier
                        | Substate::SeekingOfficerWaitForInstructedGroup,
                    ) => {
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::BeginSoldierGiveReport {
                                officer: self.base.antagonist,
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
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallYourTalk1,
                        info: StimulusInfo::Human(self.base.me),
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
            for (index, handle) in instructed.iter().copied().enumerate() {
                let soldier_seek_pos = if charly_waypoints.is_empty() {
                    if index > 0 {
                        // Everyone after the first searches the area
                        // around the point, not the point itself.
                        seek_flags &= !SeekFlags::LOCATION_FIRST;
                    }
                    seek_pos
                } else {
                    let pos = charly_waypoints[waypoint_index];
                    waypoint_index = (waypoint_index + waypoint_step) % charly_waypoints.len();
                    pos
                };
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RequestThinkResult {
                        target: handle,
                        caller: self.base.me,
                        stimulus_type: StimulusType::CallInstruction,
                        info: StimulusInfo::Hint(Hint {
                            seek_point: soldier_seek_pos,
                            seek_flags: seek_flags.bits(),
                            who_tells_me: self.base.me,
                        }),
                        continuation: ThinkResultContinuation::OfficerInstructedGroupSoldier {
                            last: index + 1 == instructed.len(),
                        },
                    },
                );
            }
            if instructed.is_empty() {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                    StimulusInfo::Human(h) => h,
                    _ => self.base.antagonist,
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
                    if report.report_type == ReportType::MissedCharly && report.charly != 0 {
                        let charly = ctx.entity_view(report.charly).unwrap_or_else(|| {
                            panic!(
                                "officer waiting for instructed group requires Charly {} view",
                                report.charly
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
                self.go_to(
                    self.base.current_state,
                    self.base.current_substate,
                    self.gather_position,
                    GotoFlags::RUN,
                    ctx,
                );
            } else {
                let officer_pos = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .map(|cs| cs.position)
                    .unwrap_or(self.gather_position);
                self.go_near(
                    self.base.current_state,
                    self.base.current_substate,
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                    self.base.face_direction(self.gather_direction, ctx);
                } else {
                    self.face_npc(self.base.antagonist, ctx);
                }
            }
            StimulusType::EventDone => {
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                                target: self.base.antagonist,
                                stimulus_type: StimulusType::CallCoordinate,
                                info: StimulusInfo::Human(self.base.me),
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
                .find(|cs| cs.handle == hint.who_tells_me)
                .map(|cs| cs.position)
                .unwrap_or(self.officers_position);
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
                if let Some(pos) = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
                    .map(|cs| cs.position)
                {
                    let dx = pos.x - self.gather_position.x;
                    let dy = pos.y - self.gather_position.y;
                    let talk_sq = (parameters_ai::AI_TALK_DISTANCE as f32)
                        * (parameters_ai::AI_TALK_DISTANCE as f32);
                    if dx * dx + dy * dy > talk_sq {
                        // Officer moved — update and retry
                        self.gather_position = pos;
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            pos,
                            parameters_ai::AI_TALK_DISTANCE,
                            GotoFlags::RUN,
                            ctx,
                        );
                    }
                }
                self.base.launch_timer(50, ctx.frame);
            }
            StimulusType::EventReachPoint => {
                let ant = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist);
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
                        self.gather_position = officer_pos;
                        self.go_near(
                            self.base.current_state,
                            self.base.current_substate,
                            officer_pos,
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
                            .push(StimulusType::EventReachPoint);
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
                // Forward talk to officer
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallYourTalk0,
                            info: StimulusInfo::None,
                        },
                    );
                }
            }
            StimulusType::EventTimer => {
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                        self.base.launch_timer(150, ctx.frame);
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
            let officer_report = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == self.base.antagonist)
                .map(|cs| cs.report_type)
                .unwrap_or(ReportType::Nothing);

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
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallReport,
                        info: StimulusInfo::Human(self.base.me),
                    });
                // Original invokes both recipient Think calls before
                // changing the caller's substate. The officer's
                // CALL_YOURTALK_1 handling can synchronously call this
                // soldier back, and that callback must still observe
                // the report-start substate.
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RequestThinkResult {
                        target: self.base.antagonist,
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
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallReport,
                        info: StimulusInfo::Human(self.base.me),
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
                let ant_substate = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == self.base.antagonist)
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
                    StimulusInfo::Human(h) => h,
                    _ => self.base.antagonist,
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
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                        target: self.base.antagonist,
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
                if !body_stuck && self.is_detecting(self.base.detected_body as HumanHandle, ctx) {
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
                        let net_obj = self.base.interesting_object;
                        let active = net_obj != 0 && ctx.entity_position(net_obj).is_some();
                        if active {
                            self.set_state(AiState::Seeking, Substate::SeekingTakingNet);
                            self.base.stop_all();
                            let owner = self.base.owner_entity_id;
                            let antagonist = Some(crate::element::EntityId::Net(
                                crate::entity_id::NetId(net_obj),
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
            let body = self.base.detected_body;
            let view = ctx.expect_entity_view(body, "taking-net body");
            let stuck = view.stuck_under_net;
            let examine = !view.is_able_to_fight && !view.stuck_under_net;
            if stuck || examine {
                // `run_to_examine_body` internally forks on
                // `stuck_under_net` and transitions into
                // SeekingNet for the net-takedown path, or
                // SeekingBody for the examine path.
                self.run_to_examine_body(body, ctx, tick, grid);
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
                if self.base.checkpoint_charly == 0 {
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
            // Inlined `missed_charly_alert()`: a ~40-line
            // sequence of recon-report updates, rank branch
            // (AlertOfficer/AlertSoldiers), and a SeekArea
            // fallback.  The "set_reported_to_officer(false)"
            // on `checkpoint_charly` writes to another NPC's
            // field; queued via
            // `pending_set_reported_to_officer` and drained
            // by the engine after the think pass.
            self.base.say(Remark::DidntFindCharly);
            let my_pos = ctx.position;
            self.base.seek_position = my_pos;
            self.base
                .my_reconnaissance_report
                .update(ReportType::MissedCharly, my_pos);
            self.base.my_reconnaissance_report.charly = self.base.checkpoint_charly;
            self.base.frame_when_enemy_detected = ctx.frame;
            // charly.set_reported_to_officer(false).
            // Queued via `pending_set_reported_to_officer`;
            // the engine drains these pairs after the think
            // pass.
            if self.base.checkpoint_charly != 0 {
                self.base
                    .outbox
                    .actor
                    .set_reported_to_officer
                    .push((self.base.checkpoint_charly as NpcHandle, false));
            }
            let alert_handled = match self.get_rank() {
                ProfileRank::Soldier => {
                    self.alert_officer(sim, my_pos, SeekFlags::CHARLY_SEEK.bits(), ctx, tick)
                }
                ProfileRank::Officer => self.alert_soldiers(
                    my_pos,
                    SeekFlags::CHARLY_SEEK.bits(),
                    global,
                    grid,
                    ctx,
                    tick,
                    AlertSoldiersFailureContinuation::SeekMissedCharly { center: my_pos },
                ),
                ProfileRank::Knight | ProfileRank::None => false,
            };
            if !alert_handled {
                // Seek yourself fallback.  Uses the FIX radius
                // if the checkpoint charly has no patrol path,
                // else PATROL radius.
                let charly_has_path = ctx
                    .expect_entity_view(
                        self.base.checkpoint_charly,
                        "missed-charly checkpoint charly",
                    )
                    .has_patrol_path;
                let radius = if charly_has_path {
                    parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
                } else {
                    parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
                };
                self.seek_area(
                    sim,
                    my_pos,
                    radius,
                    SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
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
                if charly == 0 {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestThinkResult {
                            target: charly,
                            caller: self.base.me,
                            stimulus_type: StimulusType::CallGoToOfficer,
                            info: crate::ai::StimulusInfo::Human(self.base.antagonist),
                            continuation: ThinkResultContinuation::OfficerSentCharlyToOfficer,
                        },
                    );
                }
            }
            StimulusType::EventMyTalk2 => {
                self.set_state(AiState::Seeking, Substate::SeekingLookingResurrectedCharly);
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
            self.base.outbox.actor.unalert_near_charly_seekers = Some(CharlySeekerTarget::SelfNpc);
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
                // Original: mpMe->IsDetecting(mpAntagonist). This is
                // the normal live view cone, not the 360° helper.
                if self.is_detecting(self.base.antagonist as HumanHandle, ctx) {
                    // The engine delivers this action synchronously and
                    // feeds the officer's actual Think return value into
                    // `resolve_charly_officer_report`.
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::ReportBackToOfficer {
                            officer: self.base.antagonist,
                            charly: self.base.me,
                        },
                    );
                } else {
                    // unalert_all_near_charly_seekers(me).
                    self.base.outbox.actor.unalert_near_charly_seekers =
                        Some(CharlySeekerTarget::SelfNpc);
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
                    self.base.outbox.actor.unalert_near_charly_seekers =
                        Some(CharlySeekerTarget::SelfNpc);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            StimulusType::EventReachPoint => {
                // antagonist.think(CallCoordinate, me)
                if self.base.antagonist != 0 {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallCoordinate,
                            info: crate::ai::StimulusInfo::Human(self.base.me as HumanHandle),
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
            self.base.say(Remark::CharlyDefendsHimself);
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
                    if self.base.antagonist != 0 => {
                        self.base
                            .outbox.reentrant.cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: self.base.antagonist,
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
                    crate::ai::StimulusInfo::Human(h) if h as NpcHandle == self.base.antagonist,
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
            StimulusType::EventMyTalk1 if self.base.antagonist != 0 => {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: self.base.antagonist,
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
            if self.base.antagonist != 0 {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: self.base.antagonist,
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
            if self.base.antagonist != 0 {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: self.base.antagonist,
                        stimulus_type: StimulusType::CallReport,
                        info: crate::ai::StimulusInfo::Human(self.base.me as HumanHandle),
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

    fn think_expected_attacking_event(
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
            Substate::AttackingReactiontimeTurning => {
                self.attacking_reactiontime_turning(stimulus_type, ctx)
            }

            Substate::AttackingReactiontime => {
                self.attacking_reactiontime(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingReactiontimeRunning => {
                self.attacking_reactiontime_running(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingRunningToEnemy
            | Substate::AttackingWalkingToEnemy
            | Substate::AttackingChargingEnemy => {
                self.attacking_running_to_enemy(stimulus_type, ctx, tick, grid)
            }

            Substate::AttackingOverviewLookLeft => self.attacking_overview_look_left(stimulus_type),

            Substate::AttackingOverviewLookRight => {
                self.attacking_overview_look_right(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventGaloppLoopEnd =>
            {
                self.attacking_rider_charging_approaching_on_event_galopp_loop_end(ctx, tick, grid)
            }

            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                self.attacking_rider_charging_approaching_on_event_reach_point(ctx, tick)
            }

            Substate::AttackingRiderChargingPassing
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                self.attacking_rider_charging_passing(ctx, grid)
            }

            Substate::AttackingRiderChargingGettingDistance => {
                self.attacking_rider_charging_getting_distance(stimulus_type, ctx)
            }

            Substate::AttackingRiderChargingReturning => {
                self.attacking_rider_charging_returning(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingRiderChargingApproachingBlindly => {
                self.attacking_rider_charging_approaching_blindly(stimulus_type, ctx)
            }

            Substate::AttackingSwordfight => {
                self.attacking_swordfight(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingSwordfightSpecialStrike => {
                self.attacking_swordfight_special_strike(stimulus_type, ctx)
            }

            Substate::AttackingSwordfightParade => {
                self.attacking_swordfight_parade(stimulus_type, ctx)
            }

            Substate::AttackingApproachingNewEnemy => {
                self.attacking_approaching_new_enemy(stimulus_type, ctx, tick)
            }

            Substate::AttackingMovingAroundOldEnemy => {
                self.attacking_moving_around_old_enemy(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingQuittingSwordfight => {
                self.attacking_quitting_swordfight(stimulus_type, ctx, tick)
            }

            Substate::AttackingReserve => self.attacking_reserve(stimulus_type, ctx, tick),

            Substate::AttackingLastReserve => {
                self.attacking_last_reserve(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingApproachToObserve => {
                self.attacking_approach_to_observe(stimulus_type, ctx)
            }

            Substate::AttackingObserve => {
                self.attacking_observe(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingObserveAndMove => {
                self.attacking_observe_and_move(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingTooProudToAttack => {
                self.attacking_too_proud_to_attack(sim, stimulus_type, ctx, tick)
            }

            Substate::AttackingTowerGuardAlert => {
                self.attacking_tower_guard_alert(stimulus_type, ctx, tick)
            }

            Substate::AttackingTowerGuardObserve => {
                self.attacking_tower_guard_observe(stimulus_type, ctx, tick)
            }

            Substate::AttackingBowShooting => {
                self.attacking_bow_shooting(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingBowAiming => {
                self.attacking_bow_aiming(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingBowLoading => self.attacking_bow_loading(stimulus_type, ctx),

            Substate::AttackingBowObservingLoading => {
                self.attacking_bow_observing_loading(stimulus_type, ctx)
            }

            Substate::AttackingBowObserving => {
                self.attacking_bow_observing(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingBowRunningBehindShieldBearer => self
                .attacking_bow_running_behind_shield_bearer(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingDoorFightDelay => {
                self.attacking_door_fight_delay(stimulus_type, ctx)
            }

            Substate::AttackingDoorFightLeaving => {
                self.attacking_door_fight_leaving(stimulus_type, ctx)
            }

            Substate::AttackingDoorFightTurning => {
                self.attacking_door_fight_turning(stimulus_type, ctx, tick)
            }

            Substate::AttackingDoorFightWaiting => {
                self.attacking_door_fight_waiting(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingProtectingWithShield => {
                self.attacking_protecting_with_shield(sim, stimulus_type, ctx, tick)
            }

            Substate::AttackingAdvancingWithShield => {
                self.attacking_advancing_with_shield(stimulus_type, ctx, tick, grid)
            }

            Substate::AttackingRunningToPhalanx => {
                self.attacking_running_to_phalanx(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingPhalanx => {
                self.attacking_phalanx(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingReserveOverview => {
                self.attacking_reserve_overview(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingApproachingSleepingEnemy => {
                self.attacking_approaching_sleeping_enemy(sim, stimulus_type, ctx, tick)
            }

            Substate::AttackingKillingSleepingEnemy => {
                self.attacking_killing_sleeping_enemy(stimulus_type, ctx, tick)
            }

            Substate::AttackingArcherRetireFromCombat => {
                self.attacking_archer_retire_from_combat(stimulus_type, ctx)
            }

            Substate::AttackingArcherRetireFromCombatTurn => self
                .attacking_archer_retire_from_combat_turn(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingOfficerGivingOrders => {
                self.attacking_officer_giving_orders(stimulus_type, ctx)
            }

            Substate::AttackingOfficerGivingOrdersWaiting => self
                .attacking_officer_giving_orders_waiting(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingTooProudToAttackOverview => self
                .attacking_too_proud_to_attack_overview(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingTooProudToAttackRetire => {
                self.attacking_too_proud_to_attack_retire(stimulus_type, ctx)
            }

            Substate::AttackingTooProudToAttackRetireTurn => self
                .attacking_too_proud_to_attack_retire_turn(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingTooProudToAttackApproach => self
                .attacking_too_proud_to_attack_approach(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingArcherRunOnShootingPath => {
                self.attacking_archer_run_on_shooting_path(stimulus_type, global, ctx)
            }

            Substate::AttackingArcherRunOnShootingPathFinalSprint => {
                self.attacking_archer_run_on_shooting_path_final_sprint(stimulus_type, global, ctx)
            }

            Substate::AttackingArcherRunOnShootingPathTurn => self
                .attacking_archer_run_on_shooting_path_turn(
                    sim,
                    stimulus_type,
                    global,
                    ctx,
                    tick,
                    grid,
                ),

            Substate::AttackingReactiontimeBending => {
                self.attacking_reactiontime_bending(sim, stimulus_type, global, ctx, tick, grid)
            }

            Substate::AttackingArcherWaitOnArcheryPath
            | Substate::AttackingArcherWaitOnArcheryPathBending => {
                self.attacking_archer_wait_on_archery_path(sim, stimulus_type, ctx, tick)
            }

            Substate::AttackingArcherWaitOnBendPoint => {
                self.attacking_archer_wait_on_bend_point(sim, stimulus_type, ctx, tick)
            }

            Substate::AttackingDummyBehaviour => self.attacking_dummy_behaviour(stimulus_type, ctx),

            Substate::AttackingSwordfightStepBack => {
                self.attacking_swordfight_step_back(stimulus_type, ctx)
            }

            Substate::AttackingReturnToOtherPcAfterMenacing => {
                self.attacking_return_to_other_pc_after_menacing(stimulus_type, ctx, tick)
            }

            Substate::AttackingRunningToLadder => {
                self.attacking_running_to_ladder(stimulus_type, ctx, tick, grid)
            }

            Substate::AttackingWaitingAtLadder => {
                self.attacking_waiting_at_ladder(stimulus_type, ctx, tick, grid)
            }

            Substate::AttackingRunToAvengerOnRoof => {
                self.attacking_run_to_avenger_on_roof(stimulus_type, ctx)
            }

            Substate::AttackingWaitForAvengerOnRoof => {
                self.attacking_wait_for_avenger_on_roof(sim, stimulus_type, global, ctx, tick)
            }

            // No-op group — only substates that still genuinely have no
            // handler remain here.  Explicit enumeration (no `_ =>`
            // catch-all) so adding a new `Substate` variant is a compile
            // error forcing the author to decide where it belongs.  The
            // 83 variants ported above were previously swept into this
            // group by the exhaustive-match refactor; see commit fbda9e7a
            // (AttackingSwordfightParade) for the original motivating fix.
            Substate::AttackingRiderChargingApproaching
            | Substate::AttackingRiderChargingPassing => false,

            _ => false,
        }
    }

    fn attacking_reactiontime_turning(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone || stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Attacking, Substate::AttackingReactiontime);

            // Timer depends on target's current animation and
            // distance.  Note: the inner-switch reads
            // `current_state` AFTER `set_state(Attacking,
            // ...)` above, so the STATE_ATTACKING arm always
            // wins and the `default: React(AI_MAX_...)` arm
            // is effectively dead code in this dispatch.  We
            // used to call `react(AI_MAX_ENEMY_REACTIONTIME)`
            // unconditionally, which over-delayed engagement.
            let target_view = ctx
                .entity_view(self.base.primary_target)
                .unwrap_or_else(|| {
                    panic!(
                        "primary target {} missing from entity views while reacting",
                        self.base.primary_target
                    )
                });
            let target_world = target_view.detection_position_world;
            let owner_world = ctx.self_body_position_world;
            let dx = target_world.x - owner_world.x;
            let dy =
                (target_world.y - owner_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_world.z - owner_world.z;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();

            if target_view.current_animation == crate::order::OrderType::RunningUpright {
                // Enemy running — react fast to intercept.
                self.base.launch_timer(
                    parameters_ai::AI_RUNNING_ENEMY_REACTIONTIME as u32,
                    ctx.frame,
                );
            } else if distance < 30.0 {
                // Aaaaagh, he is too close!
                self.base.launch_timer(1, ctx.frame);
            } else {
                self.base
                    .launch_timer(parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32, ctx.frame);
            }
        }
        false
    }

    fn attacking_reactiontime(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        tracing::trace!(
            me = self.base.me,
            frame = ctx.frame,
            ?stimulus_type,
            timer_ring = self.base.when_does_timer_ring,
            "reactiontime arm: stimulus received"
        );
        if stimulus_type == StimulusType::EventTimer {
            // Archer leaning out has a special branch: re-init
            // enemy list, transition to ReactiontimeBending,
            // queue EquipBowDown command.  Otherwise fall
            // through to the standard `i_am_in_trouble` +
            // `battle_decisions`.
            if ctx.posture == crate::element::Posture::LeaningOut && self.is_archer() {
                self.reinitialize_them_list(ctx, tick);
                self.set_state(AiState::Attacking, Substate::AttackingReactiontimeBending);
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::EquipBowDown);
            } else {
                self.i_am_in_trouble(self.base.primary_target);
                self.battle_decisions(sim, global, ctx, tick, grid);
            }
        }
        false
    }

    fn attacking_reactiontime_running(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer
            || stimulus_type == StimulusType::EventReachPoint
        {
            self.base.stop_all();
            self.i_am_in_trouble(self.base.primary_target);
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    fn attacking_running_to_enemy(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint | StimulusType::EventTimer => {
                let distance = {
                    let dx = self.base.seek_position.x - ctx.position.x;
                    let dy = (self.base.seek_position.y - ctx.position.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    (dx * dx + dy * dy).sqrt()
                };
                self.reconsider_enemy_approach(
                    stimulus_type == StimulusType::EventReachPoint,
                    distance,
                    ctx,
                    tick,
                    grid,
                );
            }
            _ => {}
        }
        false
    }

    fn attacking_overview_look_left(&mut self, stimulus_type: StimulusType) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(AiState::Attacking, Substate::AttackingOverviewLookRight);
            // Look RIGHT.
            self.base.outbox.actor.look_sidewards = Some(LookDirection::Right);
        }
        false
    }

    fn attacking_overview_look_right(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            // Look-sidewards finished — short delay before deciding.
            StimulusType::EventDone => {
                self.base.launch_timer(10, ctx.frame);
            }
            // Timer fires → BattleDecisions.
            StimulusType::EventTimer => {
                self.battle_decisions(sim, global, ctx, tick, grid);
            }
            _ => {}
        }
        false
    }

    // ── Rider charging substates ──

    // Approaching: rider is running toward the enemy with RIDER_CHARGE flag.
    // On GALOPP_LOOP_END: check if we can begin the actual charge.
    // On REACHPOINT: we arrived at the approach position; overview battle.

    fn attacking_rider_charging_approaching_on_event_galopp_loop_end(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // If can't charge, fall back to normal attack
        if !self.maybe_make_rider_attack(ctx, tick, grid) {
            self.set_state(AiState::Attacking, Substate::AttackingRunningToEnemy);
            let distance = {
                let dx = self.base.seek_position.x - ctx.position.x;
                let dy = (self.base.seek_position.y - ctx.position.y)
                    * crate::position_interface::INVERSE_ASPECT_RATIO;
                (dx * dx + dy * dy).sqrt()
            };
            self.reconsider_enemy_approach(true, distance, ctx, tick, grid);
        }
        false
    }

    fn attacking_rider_charging_approaching_on_event_reach_point(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Arrived at approach point
        self.get_battle_overview(0, ctx, tick);
        false
    }

    // Passing: rider is doing the actual charge pass through the enemy.
    // On REACHPOINT: charge pass is done, get distance for reattack.

    fn attacking_rider_charging_passing(
        &mut self,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Transition to getting distance
        self.set_state(
            AiState::Attacking,
            Substate::AttackingRiderChargingGettingDistance,
        );
        if let Some(goal) = self.get_good_rider_reattack_goal(ctx, grid) {
            // Ride away for reattack distance
            self.go_to(
                self.base.current_state,
                self.base.current_substate,
                goal,
                GotoFlags::RUN,
                ctx,
            );
        } else {
            // Attack from here
            self.base.fire_self_stimulus(StimulusType::EventReachPoint);
        }
        false
    }

    // GettingDistance: rider is riding away after the charge pass.
    // On REACHPOINT: arrived at retreat point, turn to face enemy.

    fn attacking_rider_charging_getting_distance(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            // Face the seek position (enemy last known pos)
            self.base
                .face_position_3d_with_ctx(self.base.seek_position, ctx);
            self.set_state(
                AiState::Attacking,
                Substate::AttackingRiderChargingReturning,
            );
        }
        false
    }

    // Returning: rider has turned to face enemy, waiting for turn to complete.
    // On EVENT_DONE: turn complete, try reattacking.

    fn attacking_rider_charging_returning(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.rider_reattack(sim, global, ctx, tick, grid);
        }
        false
    }

    // ApproachingBlindly: rider lost sight of all enemies, riding
    // to last known position.
    // On REACHPOINT: arrived, look around wondering.

    fn attacking_rider_charging_approaching_blindly(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            // Enter wondering state
            self.set_state(AiState::Wondering, Substate::WonderingLooking1);
            self.base.launch_timer(30, ctx.frame);
        }
        false
    }

    // `AttackingSwordfight` matches `EventReachPoint` /
    // `EventDone` / `EventTimer`, clears the emoticon, calls
    // `reconsider_swordfight`, and (if still in the same
    // substate) says `CombatInsult`.
    //
    // Special-strike work occupies its own legacy substate and
    // therefore does not enter this ordinary swordfight arm. The
    // lifecycle latch remains as a cancellation guard; reconciliation
    // restores this state and relaunches the 20-frame heartbeat.

    fn attacking_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if matches!(
            stimulus_type,
            StimulusType::EventTimer | StimulusType::EventDone | StimulusType::EventReachPoint
        ) && !self.pending_special_strike
        {
            // SetEmoticon(None).
            self.base.set_emoticon(EmoticonType::None);
            self.reconsider_swordfight(sim, false, global, ctx, tick, grid);
            // Original proposes a strike inside ReconsiderSwordfight.
            // A successful proposal first changes to SpecialStrike,
            // suppressing this taunt. Our proposal needs engine state,
            // so defer the decision across that owner-local boundary.
            if self.base.current_substate == Substate::AttackingSwordfight {
                if self.pending_sword_strike_consideration {
                    self.pending_combat_insult_after_strike_consideration = true;
                } else {
                    self.base.say(Remark::CombatInsult);
                }
            }
        }
        false
    }

    // A completed (or timed-out) strike returns to the ordinary
    // swordfight heartbeat. This is an observable legacy substate,
    // not merely an internal sequence latch.

    fn attacking_swordfight_special_strike(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if matches!(
            stimulus_type,
            StimulusType::EventDone | StimulusType::EventTimer
        ) {
            self.finish_special_strike(ctx.frame);
        }
        false
    }

    // `AttackingSwordfightParade`: on `EventTimer`, end the
    // parry only if it is still active, transition back to
    // `AttackingSwordfight`, and re-launch the 20-tick heartbeat.
    // Original checks exactly `RHACTIONSTATE_PARRYING_SWORD` before
    // queuing StopParrySword. Queuing it after the actor has already
    // returned to WaitingSword is not a harmless no-op: terminating
    // that element emits a synchronous EventDone and spuriously
    // reconsiders the swordfight.

    fn attacking_swordfight_parade(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if ctx.self_action_state == crate::element::ActionState::ParryingSword {
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::StopParrySword);
            }
            self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
            self.base.launch_timer(20, ctx.frame);
        }
        false
    }

    // Reached position near new enemy.

    fn attacking_approaching_new_enemy(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            let sword_range = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.sword_range_default)
                .unwrap_or(self.sword_range);
            let target = self
                        .find_fighter(self.base.primary_target, tick)
                        .unwrap_or_else(|| {
                            panic!(
                                "AttackingApproachingNewEnemy primary target {} is missing its required fighter snapshot",
                                self.base.primary_target
                            )
                        });
            if tick.primary_target_snapshot_handle != self.base.primary_target {
                panic!(
                    "AttackingApproachingNewEnemy primary target {} does not match tick snapshot handle {}",
                    self.base.primary_target, tick.primary_target_snapshot_handle
                );
            }
            let target_live_position = tick.primary_target_live_position.unwrap_or_else(|| {
                        panic!(
                            "AttackingApproachingNewEnemy primary target {} is missing its literal position",
                            self.base.primary_target
                        )
                    });
            let owner_live_position = tick.owner_live_position.unwrap_or_else(|| {
                panic!(
                    "AttackingApproachingNewEnemy owner {} is missing its literal position",
                    self.base.me
                )
            });
            let close_enough = approaching_new_enemy_is_close_enough(
                &target_live_position,
                target.elevation,
                &owner_live_position,
                ctx.elevation,
                sword_range,
            );

            if close_enough {
                self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                self.base.launch_timer(20, ctx.frame);
                self.base.outbox.actor.set_principal = Some(self.base.primary_target);
            } else {
                // Re-approach
                self.base
                    .go_near(target.position, sword_range as i32, GotoFlags::RUN, ctx);
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
                    self.base.launch_timer(20, ctx.frame);
                    self.base.outbox.actor.set_principal = Some(self.base.primary_target);
                }
            }
        }
        false
    }

    // Reached position around old enemy.

    fn attacking_moving_around_old_enemy(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
            self.base.launch_timer(20, ctx.frame);
            self.reconsider_swordfight(sim, false, global, ctx, tick, grid);
        }
        false
    }

    // Quitting swordfight timer.

    fn attacking_quitting_swordfight(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // Original tests `_ANY_SWORD_ACTIONSTATE_`, not whether
            // the opponent list is still populated. EndSwordfight
            // removes opponents before the current sword animation
            // necessarily finishes, so the action state remains the
            // authoritative completion gate.
            if ctx.self_action_state.is_sword() {
                // Still in a sword action state — retry quit and wait.
                self.end_swordfight(ctx, tick);
                self.base.launch_timer(3, ctx.frame);
            } else {
                // Left sword state — proceed to battle overview.
                self.get_battle_overview(0x0000, ctx, tick);
            }
        }
        false
    }

    fn attacking_reserve(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            // Fall-through to CallCoordinate: walk the us-list built
            // by the last BattleDecisions, pick the soldiers still in
            // AttackingReserve and send each a CallCoordinate so they
            // all begin their overview together.  The scan is over the
            // us-list, not the whole camp roster: only allies this
            // soldier actually perceived during its own decision pass
            // participate, and they are visited in us-list order.
            StimulusType::EventTimer => {
                let me = self.base.me;
                let friends_to_coord: Vec<NpcHandle> = self
                    .base
                    .list_us
                    .iter()
                    .filter(|&&handle| handle != me)
                    .filter(|&&handle| {
                        tick.camp_soldiers.iter().any(|cs| {
                            cs.handle == handle && cs.ai_substate == Substate::AttackingReserve
                        })
                    })
                    .copied()
                    .collect();
                for target in friends_to_coord {
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                            target,
                            stimulus_type: StimulusType::CallCoordinate,
                            info: crate::ai::StimulusInfo::Human(me),
                        },
                    );
                }
                // Fall through to CallCoordinate arm.
                self.reinitialize_them_list(ctx, tick);
                self.base.set_emoticon(EmoticonType::None);
                self.set_state(AiState::Attacking, Substate::AttackingReserveOverview);
                self.base.launch_timer(20, ctx.frame);
            }
            StimulusType::CallCoordinate => {
                self.reinitialize_them_list(ctx, tick);
                self.base.set_emoticon(EmoticonType::None);
                self.set_state(AiState::Attacking, Substate::AttackingReserveOverview);
                self.base.launch_timer(20, ctx.frame);
            }
            _ => {}
        }
        false
    }

    // Last reserve: BattleDecisions on timer
    // (AttackingReserveOverview is below).

    fn attacking_last_reserve(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    // Approach-to-observe: on EventTimer, face primary
    // target, stop, queue EnterSwordfight (no opponent —
    // sword-raise only), transition to Observe, clear
    // emoticon, launch 50-tick timer.

    fn attacking_approach_to_observe(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if self.base.primary_target != 0 {
                self.base
                    .set_direction_toward_entity(self.base.primary_target, ctx);
            }
            self.base.stop_all();
            // Launch EnterSwordfight with opponent=0 (just
            // rises the sword pose).
            self.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::RaiseSword);
            self.base.outbox.actor.enter_swordfight_jump_line = None;
            self.set_state(AiState::Attacking, Substate::AttackingObserve);
            self.base.set_emoticon(EmoticonType::None);
            self.base.launch_timer(50, ctx.frame);
        }
        false
    }

    fn attacking_observe(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.reconsider_swordfight_observation(sim, global, ctx, tick, grid);
        }
        false
    }

    // ReconsiderSwordfightObservation: reached the observe-and-move
    // destination — immediately reconsider the current swordfight.

    fn attacking_observe_and_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.reconsider_swordfight_observation(sim, global, ctx, tick, grid);
        }
        false
    }

    // TooProud entry: reinit list, clear emoticon, transition
    // to Overview, 1/16 chance of LookSidewards else 20-tick
    // timer.

    fn attacking_too_proud_to_attack(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.reinitialize_them_list(ctx, tick);
            self.base.set_emoticon(EmoticonType::None);
            self.set_state(
                AiState::Attacking,
                Substate::AttackingTooProudToAttackOverview,
            );
            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::TooProudLook, 0..16) == 0 {
                self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
            } else {
                self.base.launch_timer(20, ctx.frame);
            }
        }
        false
    }

    fn attacking_tower_guard_alert(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.tower_guard_call_alert(self.base.seek_position, ctx, tick);
            // TowerGuardCallAlert is synchronous in the reference;
            // suspend this tail until every recipient has handled the
            // cry. Their Think calls can change alert status, and the
            // guard's next BattleDecisions may enter SeekArea and scan
            // those live recipient states immediately.
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::ResumeTowerGuardBattleDecisions {
                    caller: self.base.me,
                },
            );
        }
        false
    }

    fn attacking_tower_guard_observe(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // The observing tower guard takes a fresh battle
            // overview rather than re-deciding directly: that
            // rebuilds the them-list, drops the task priority back
            // to the minimum and starts the look-left/look-right
            // sweep instead of immediately re-entering the same
            // observe substate.
            self.get_battle_overview(0, ctx, tick);
        }
        false
    }

    // Shooting state:
    // CallCoordinate → StopAll + BattleDecisions; EventDone →
    // ReinitializeThemList + BattleDecisions (NO state
    // transition; BattleDecisions decides whether to reload,
    // observe, etc.).

    fn attacking_bow_shooting(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventDone => {
                self.reinitialize_them_list(ctx, tick);
                self.battle_decisions(sim, global, ctx, tick, grid);
            }
            StimulusType::CallCoordinate => {
                self.base.stop_all();
                self.battle_decisions(sim, global, ctx, tick, grid);
            }
            _ => {}
        }
        false
    }

    // Aiming: clear emoticon, gate on tower-guard /
    // archer-distance: if safe, transition to Shooting +
    // ShootArrowAt; else clear enemy_seen_below +
    // BattleDecisions.

    fn attacking_bow_aiming(
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
                self.base.set_emoticon(EmoticonType::None);
                let safe_to_shoot = self.tower_guard
                    || !self.archer_is_too_near_to_enemy(
                        &ctx.position,
                        self.base.primary_target,
                        ctx,
                        tick,
                    );
                if safe_to_shoot {
                    self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                    self.shoot_arrow_at(self.base.primary_target, ctx, tick);
                } else {
                    self.enemy_seen_below = false;
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }
            _ => {}
        }
        false
    }

    // Loading complete: transition to Aiming, launch
    // AIMING_TIME_FORMULA timer.

    fn attacking_bow_loading(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        match stimulus_type {
            StimulusType::EventDone => {
                self.set_state(AiState::Attacking, Substate::AttackingBowAiming);
                // `AIMING_TIME_FORMULA = (110 -
                // shooting_ability) / 2`.  Same formula as
                // the Decision::Shoot site below — uses the
                // soldier's modified shooting ability (with
                // alcohol penalty), not IQ.
                let aim_time = ((110u32).saturating_sub(self.get_shooting_ability(ctx) as u32)) / 2;
                self.base.launch_timer(aim_time.max(5), ctx.frame);
            }
            _ => {}
        }
        false
    }

    // Observing-loading: on EventDone, transition to
    // BowObserving + 50-tick timer.

    fn attacking_bow_observing_loading(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventDone => {
                self.set_state(AiState::Attacking, Substate::AttackingBowObserving);
                self.base.launch_timer(50, ctx.frame);
            }
            _ => {}
        }
        false
    }

    // Observing: on timer, posture-conditional branch
    // (LeaningOut → ReinitializeThemList; else StopAll),
    // then BattleDecisions.

    fn attacking_bow_observing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if ctx.posture == crate::element::Posture::LeaningOut {
                self.reinitialize_them_list(ctx, tick);
            } else {
                self.base.stop_all();
            }
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    // Archer running to cover position behind a shield bearer.

    fn attacking_bow_running_behind_shield_bearer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                // Arrived behind shield bearer — turn to face target.
                if self.base.primary_target != 0 {
                    // Original: `Face(mpPrimaryTarget)`. The element
                    // overload forwards both `Position(target)` and
                    // the target's truncated elevation; dropping the
                    // latter can select an adjacent sector.
                    self.base.face_entity(self.base.primary_target, ctx);
                }
            }
            StimulusType::EventDone => {
                // Facing done — short delay to recenter viewcone
                // before re-evaluating: `launch_timer(5)` +
                // `focus(primary_target)`.
                self.base.launch_timer(5, ctx.frame);
                if self.base.primary_target != 0 {
                    self.base.outbox.actor.set_focus(self.base.primary_target);
                }
            }
            StimulusType::EventTimer => {
                // Re-evaluate: normally shoot at primary target.
                self.reinitialize_them_list(ctx, tick);
                self.battle_decisions(sim, global, ctx, tick, grid);
            }
            _ => {}
        }
        false
    }

    fn attacking_door_fight_delay(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.set_state(AiState::Attacking, Substate::AttackingDoorFightLeaving);
            self.base
                .go_to(self.base.seek_position, GotoFlags::RUN, ctx);
        }
        false
    }

    fn attacking_door_fight_leaving(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(AiState::Attacking, Substate::AttackingDoorFightTurning);
            self.base.face_direction(self.gather_direction, ctx);
        }
        false
    }

    // Door-fight turning complete: if no target, wait 150
    // ticks; else BeginSwordfight.

    fn attacking_door_fight_turning(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            if self.base.primary_target == 0 {
                self.set_state(AiState::Attacking, Substate::AttackingDoorFightWaiting);
                self.base.launch_timer(150, ctx.frame);
            } else {
                self.begin_swordfight(ctx, tick);
            }
        }
        false
    }

    fn attacking_door_fight_waiting(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.reinitialize_them_list(ctx, tick);
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    // ============ PHALANX / SHIELD-BEARER ============

    fn attacking_protecting_with_shield(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Stand in place with shield, timer-driven re-evaluation
        if stimulus_type == StimulusType::EventTimer {
            let my_action = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.action_state)
                .unwrap_or_default();

            if !my_action.is_shield() {
                // Reestablish shield state
                let (target_pos, target_elevation) = self
                    .find_fighter(self.base.primary_target, tick)
                    .map(|f| (f.position, f.elevation as f32))
                    .unwrap_or((ctx.position, ctx.elevation));
                self.base.raise_shield(target_pos, target_elevation);
                self.base.launch_timer(20, ctx.frame);
            } else if self.left_combat_neighbour != 0 || self.right_combat_neighbour != 0 {
                // You should be doing phalanx stuff
                self.set_state(AiState::Attacking, Substate::AttackingPhalanx);
                // Original calls `mpMe->Focus(mpPrimaryTarget)` here.
                // Focus locks the NPC view cone; it does not launch an
                // actor Face/Turn command.
                self.base.outbox.actor.set_focus(self.base.primary_target);
                self.base.launch_timer(5, ctx.frame);
            } else if self.archer_behind_me != 0 {
                // Protecting an archer — update direction and stay
                let target_pos = self
                    .find_fighter(self.base.primary_target, tick)
                    .map(|f| f.position)
                    .unwrap_or(ctx.position);
                let dx = target_pos.x - ctx.position.x;
                let dy = target_pos.y - ctx.position.y;
                let dir = vec_to_sector(dx, dy);
                self.base.set_direction_goal(dir);
                self.base.outbox.actor.refresh_shield = true;
                self.base.launch_timer(30, ctx.frame);
            } else {
                // Check if enemy is still dangerous
                let target = if self.base.primary_target != 0 {
                    self.base.primary_target
                } else {
                    self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick)
                };
                if target != 0 {
                    let target_is_bow = self
                        .find_fighter(target, tick)
                        .map(|f| f.action_state.is_bow())
                        .unwrap_or(false);
                    if target_is_bow {
                        // Still danger
                        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ShieldAdvance, 0..4)
                            == 0
                        {
                            // Lower shield to advance
                            self.set_state(
                                AiState::Attacking,
                                Substate::AttackingAdvancingWithShield,
                            );
                            self.base.lower_shield();
                        } else {
                            self.base.launch_timer(10, ctx.frame);
                        }
                    } else {
                        // Danger is over
                        self.get_battle_overview(0, ctx, tick);
                    }
                } else {
                    self.get_battle_overview(0, ctx, tick);
                }
            }
        }
        false
    }

    fn attacking_advancing_with_shield(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Step forward shield-first
        match stimulus_type {
            StimulusType::EventDone => {
                // Shield lowered, run some steps forward
                let target_pos = self
                    .find_fighter(self.base.primary_target, tick)
                    .map(|f| f.position)
                    .unwrap_or(ctx.position);
                self.go_near(
                    self.base.current_state,
                    self.base.current_substate,
                    target_pos,
                    archer::MIN_PROTECT_ARROW_DISTANCE / 2,
                    GotoFlags::RUN,
                    ctx,
                );
                self.base.launch_timer(10, ctx.frame);
            }
            StimulusType::EventTimer if !self.refresh_arrow_protection(false, ctx, tick, grid) => {
                self.get_battle_overview(0x0001, ctx, tick);
            }
            _ => {}
        }
        false
    }

    fn attacking_running_to_phalanx(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Run to phalanx slot, then raise shield
        match stimulus_type {
            StimulusType::EventReachPoint => {
                // Arrived — face the shield direction
                self.base.face_direction(self.shield_bearer_direction, ctx);
            }
            StimulusType::EventDone => {
                // Turning done — get primary target from neighbours
                let target = self
                    .phalanx_neighbour_primary_target(tick)
                    .unwrap_or_else(|| {
                        self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick)
                    });

                if target != 0 {
                    self.base.primary_target = target;
                    self.set_state(AiState::Attacking, Substate::AttackingPhalanx);
                    let (target_pos, target_elevation) = self
                        .find_fighter(target, tick)
                        .map(|f| (f.position, f.elevation as f32))
                        .unwrap_or((ctx.position, ctx.elevation));
                    self.base.raise_shield(target_pos, target_elevation);
                    // Original follows the RaiseShield launch with
                    // `Focus(mpPrimaryTarget)`, not an actor turn.
                    self.base.outbox.actor.set_focus(target);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.battle_decisions(sim, global, ctx, tick, grid);
                }
            }
            _ => {}
        }
        false
    }

    fn attacking_phalanx(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Stand in formation, reconsider periodically
        match stimulus_type {
            StimulusType::EventTimer => {
                let my_action = self
                    .find_fighter(self.base.me, tick)
                    .map(|f| f.action_state)
                    .unwrap_or_default();

                tracing::trace!(
                    target: "robin_engine::ai_enemy::phalanx",
                    me = self.base.me,
                    frame = ctx.frame,
                    ?my_action,
                    primary = self.base.primary_target,
                    "phalanx timer"
                );
                if !my_action.is_shield() && self.base.primary_target != 0 {
                    // Reestablish shield state
                    let (target_pos, target_elevation) = self
                        .find_fighter(self.base.primary_target, tick)
                        .map(|f| (f.position, f.elevation as f32))
                        .unwrap_or((ctx.position, ctx.elevation));
                    self.base.raise_shield(target_pos, target_elevation);
                    self.base.launch_timer(20, ctx.frame);
                } else if !self.reconsider_phalanx(sim, global, ctx, tick, grid) {
                    if self.base.primary_target != 0 {
                        // No phalanx correction — maybe correct direction
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        let dx = target_pos.x - ctx.position.x;
                        let dy = target_pos.y - ctx.position.y;
                        let dir = vec_to_sector(dx, dy);
                        self.base.set_direction_goal(dir);
                        self.base.outbox.actor.refresh_shield = true;
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        // No flags: a phalanx member that lost its
                        // primary target takes the plain overview,
                        // not the fast swordfight-neighbours one.
                        self.get_battle_overview(0, ctx, tick);
                    }
                }
                // else: reconsider_phalanx changed substate
            }
            StimulusType::CallInstruction => {
                // Received new position instruction from phalanx leader
                tracing::trace!(
                    target: "robin_engine::ai_enemy::phalanx",
                    me = self.base.me,
                    frame = ctx.frame,
                    gather = ?self.gather_position,
                    direction = self.gather_direction,
                    "phalanx CallInstruction: leaving AttackingPhalanx to run to new slot"
                );
                self.shield_bearer_direction = self.gather_direction;
                self.base.seek_position = self.gather_position;
                self.set_state(AiState::Attacking, Substate::AttackingRunningToPhalanx);
                self.base
                    .go_to(self.base.seek_position, GotoFlags::RUN, ctx);

                // Notify archer behind us to re-evaluate, but
                // only if they're actively shooting/loading/aiming.
                if self.archer_behind_me != 0 {
                    let archer_in_bow = self
                        .find_fighter(self.archer_behind_me, tick)
                        .map(|f| {
                            let s = f.current_substate;
                            s == Substate::AttackingBowShooting as u32
                                || s == Substate::AttackingBowLoading as u32
                                || s == Substate::AttackingBowAiming as u32
                        })
                        .unwrap_or(false);
                    if archer_in_bow {
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::SendStimulus {
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                                target: self.archer_behind_me,
                                stimulus_type: StimulusType::CallCoordinate,
                                info: StimulusInfo::None,
                            },
                        );
                    }
                }
            }
            _ => {}
        }
        false
    }

    // ============ FLEEING ============
    // The malignity arm adds a single tweak (reset of
    // `fleeing_seen_enemy_counter` on the PANIC arm when the
    // panic is over) and then falls through into
    // `think_expected_event_common_stuff`, which owns the
    // actual panic/hide/door/hiding state machine.

    fn attacking_reserve_overview(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    // Approaching a sleeping PC/NPC: reach → face target; done →
    // kill or menace depending on coma/distance.

    fn attacking_approaching_sleeping_enemy(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.base.face_entity(self.base.primary_target, ctx);
            }
            StimulusType::EventDone => {
                // Decision tree:
                //   if primary_target unconscious {
                //     if is_pc && in_coma {
                //       if pc.guard == None { menace }
                //       else { ReturnToDuty }
                //     } else if distance > 40 { GoNear 20 RUN }
                //     else { SUBSTATE_KILLING_SLEEPING_ENEMY +
                //            SWORDSTRIKE_DOWN on primary_target }
                //   } else { GetBattleOverview }
                let view =
                    ctx.expect_entity_view(self.base.primary_target, "approach-sleeping target");
                let target_pos = Some(view.position);
                let target_unconscious = view.is_unconscious;
                let target_is_pc = view.is_pc;
                let target_in_coma = view.in_coma;
                let target_guard = view.guard;

                if !target_unconscious {
                    self.get_battle_overview(0, ctx, tick);
                } else if target_is_pc && target_in_coma && target_guard.is_none() {
                    // Coma/menace branch — PC is in coma and not yet
                    // guarded.
                    self.base.stop_all();
                    self.set_state(AiState::Menacing, Substate::MenacingPcInComa);
                    if self.is_vip {
                        // VIP variant — launch an EnterSwordfight against
                        // the guarded PC to trigger the menace-variant
                        // sword draw.
                        self.base.outbox.actor.enter_swordfight =
                            Some(EnterSwordfightRequest::Engage(self.base.primary_target));
                        self.base.outbox.actor.enter_swordfight_jump_line = None;
                    } else {
                        // Normal variant — say, launch StartMenace
                        // command, set guard.
                        self.base.say(Remark::MenacesPcInComa);
                        self.base
                            .outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::StartMenace);
                        // SetGuardedPC( pPC ) — assigns both the
                        // soldier's guarded_pc and the PC's reciprocal
                        // guard.
                        self.set_guarded_pc(Some(crate::entity_id::PcId(self.base.primary_target)));
                    }
                    self.base.launch_timer(20, ctx.frame);
                } else if target_is_pc && target_in_coma && target_guard.is_some() {
                    // PC already menaced by another guard — go home.
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                } else if let Some(p) = target_pos {
                    let dx = (p.x - ctx.position.x).abs();
                    let dy = (p.y - ctx.position.y).abs();
                    if dx.max(dy) > 40.0 {
                        // GoNear(target, 20, GOTO_RUN).
                        // The kill-sleeping substate itself re-fires
                        // on REACHPOINT/DONE, so route through
                        // Approaching (same substate we're in) by
                        // issuing the move via go_near-on-self.
                        self.go_near(
                            AiState::Attacking,
                            Substate::AttackingApproachingSleepingEnemy,
                            p,
                            20,
                            crate::ai::GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // Close enough — switch to KillingSleepingEnemy
                        // and launch SwordstrikeDown on the target.
                        self.set_state(AiState::Attacking, Substate::AttackingKillingSleepingEnemy);
                        self.base.stop_all();
                        use crate::element::Command;
                        use crate::sequence::{Sequence, SequenceElement};
                        let owner = self.base.owner_entity_id;
                        let antagonist =
                            Some(ctx.entity_id(self.base.primary_target).unwrap_or_else(|| {
                                panic!(
                                    "sleeping-enemy target {} has no typed live entity view",
                                    self.base.primary_target
                                )
                            }));
                        let mut seq = Sequence::new();
                        seq.append_element(SequenceElement::new_interaction(
                            1,
                            Command::SwordstrikeDown,
                            owner,
                            antagonist,
                        ));
                        self.base.outbox.actor.launch_sequences.push(seq);
                    }
                } else {
                    self.get_battle_overview(0, ctx, tick);
                }
            }
            _ => {}
        }
        false
    }

    // Killing sleeping enemy done: say REMARK_KILLED_ADVERSARY and
    // overview.

    fn attacking_killing_sleeping_enemy(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.base.say(Remark::KilledAdversary);
            self.get_battle_overview(0, ctx, tick);
        }
        false
    }

    // Archer retires from combat, then turns (fast) to primary
    // target or seek.

    fn attacking_archer_retire_from_combat(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(
                AiState::Attacking,
                Substate::AttackingArcherRetireFromCombatTurn,
            );
            // Cheat-face the primary target if still known, else the
            // stored seek position. Original passes `true` to both
            // Face overloads, selecting RHCOMMAND_TURN_FAST.
            if self.base.primary_target != 0 {
                self.base.face_entity_fast(self.base.primary_target, ctx);
            } else {
                self.base
                    .face_position_fast_with_ctx(self.base.seek_position, ctx);
            }
        }
        false
    }

    // Done turning: re-engage via BattleDecisions or overview.

    fn attacking_archer_retire_from_combat_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // IsDetecting180Degrees(primary_target)?
            //   BattleDecisions : GetBattleOverview.
            if self.base.primary_target != 0
                && self.is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
            {
                // Original restores a still-visible primary target to
                // the persistent Them list before making the next
                // tactical decision.  The target can have fallen out
                // of that list during the retreat; omitting this made
                // BattleDecisions take its no-enemy SeekArea branch.
                if !self.list_them.contains(&self.base.primary_target) {
                    self.list_them.push(self.base.primary_target);
                }
                self.battle_decisions(sim, global, ctx, tick, grid);
            } else {
                self.get_battle_overview(0, ctx, tick);
            }
        }
        false
    }

    // Officer giving orders done: transition to waiting, mark
    // friends alerted.

    fn attacking_officer_giving_orders(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.set_state(
                AiState::Attacking,
                Substate::AttackingOfficerGivingOrdersWaiting,
            );
            self.base.friends_are_alerted = true;
            self.base.launch_timer(20, ctx.frame);
        }
        false
    }

    // Waiting after giving orders: recompute `them` list, either
    // battle or widen seek radius.

    fn attacking_officer_giving_orders_waiting(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.reinitialize_them_list(ctx, tick);
            if !self.list_them.is_empty() {
                self.battle_decisions(sim, global, ctx, tick, grid);
            } else {
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
        false
    }

    // Too-proud overview: on done, focus target and short timer; on
    // timer, BattleDecisions and maybe say
    // REMARK_PROUD_FINALLY_FIGHT.

    fn attacking_too_proud_to_attack_overview(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventDone => {
                if self.base.primary_target != 0 {
                    self.base.outbox.actor.set_focus(self.base.primary_target);
                }
                self.base.launch_timer(5, ctx.frame);
            }
            StimulusType::EventTimer => {
                // BattleDecisions, then if the resulting substate is a
                // swordfight (VIP variant says VIP_REMARK, otherwise
                // REMARK_PROUD_FINALLY_FIGHT).
                self.battle_decisions(sim, global, ctx, tick, grid);
                if self.base.current_substate.is_any_swordfight() {
                    let remark = if self.is_vip {
                        Remark::VipProudFinallyFight
                    } else {
                        Remark::ProudFinallyFight
                    };
                    self.base.say(remark);
                }
            }
            _ => {}
        }
        false
    }

    // Retiring: reach point triggers fast face-turn to seek
    // position.

    fn attacking_too_proud_to_attack_retire(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(
                AiState::Attacking,
                Substate::AttackingTooProudToAttackRetireTurn,
            );
            self.base
                .face_position_3d_with_ctx(self.base.seek_position, ctx);
        }
        false
    }

    // Finished turning: re-engage with BattleDecisions or
    // GetBattleOverview.

    fn attacking_too_proud_to_attack_retire_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            if self.base.primary_target != 0
                && self.is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
            {
                self.battle_decisions(sim, global, ctx, tick, grid);
            } else {
                self.get_battle_overview(0, ctx, tick);
            }
        }
        false
    }

    // Approach finished: BattleDecisions or overview based on
    // 180-degree detection.

    fn attacking_too_proud_to_attack_approach(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            if self.base.primary_target != 0
                && self.is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
            {
                self.battle_decisions(sim, global, ctx, tick, grid);
            } else {
                self.get_battle_overview(0, ctx, tick);
            }
        }
        false
    }

    // Note: `AttackingRiderChargingApproaching` /
    // `AttackingRiderChargingPassing` were previously duplicated
    // here with a "rider infra not ported" stub.  The real port
    // lives further up in this same match (see
    // `Substate::AttackingRiderChargingApproaching` around the
    // rider-charging substates block), which takes precedence;
    // the stubs were dead arms.  Removed.

    // Archer running on archery path: iterate waypoints, skip
    // occupied, occupy the first free shooting point.

    fn attacking_archer_run_on_shooting_path(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            // do/while loop across archery waypoints.  Bound it
            // explicitly to the current sector's waypoint count so
            // a pathological owner-map can't spin forever.
            let sector_idx = self.my_archery_sector.unwrap_or_else(|| {
                panic!(
                    "archer {} running on shooting path has no archery sector",
                    self.base.me
                )
            });
            let max_iters = global
                .archery_sectors
                .get(sector_idx as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "archer {} references missing archery sector {}",
                        self.base.me, sector_idx
                    )
                })
                .points
                .len()
                + 1;
            let mut resolved = false;
            for _ in 0..max_iters {
                self.archery_path_increment_waypoint();
                let point = self.archery_path_get_waypoint(global);
                let Some(point) = point else {
                    panic!(
                        "archer {} reached the unsupported end of archery path in sector {}",
                        self.base.me, sector_idx
                    );
                };
                if !point.is_shooting_point {
                    // GoTo next entry waypoint, RUN | DONTSTOP.
                    self.base.go_to(
                        point.position,
                        crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
                        ctx,
                    );
                    resolved = true;
                    break;
                }
                // Is it free (or already owned by me)?
                let me =
                    crate::entity_id::EntityId::Soldier(crate::entity_id::SoldierId(self.base.me));
                let owner = point.owner;
                if owner.is_none() || owner == Some(me) {
                    // Occupy + transition + GoTo.
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingArcherRunOnShootingPathFinalSprint,
                    );
                    // SetMyShootingPoint clears the prior point's
                    // owner and writes ours on the new one.
                    // `archery_path_get_waypoint` is a pure read,
                    // so `my_archery_point_index` identifies this
                    // point directly.
                    let pt_idx = u16::from(self.my_archery_point_index);
                    self.set_my_shooting_point(global, Some((sector_idx, pt_idx)));
                    self.base
                        .go_to(point.position, crate::ai::GotoFlags::RUN, ctx);
                    resolved = true;
                    break;
                }
                // Otherwise: occupied shooting point — skip
                // (loop iterates through archery_path_get_waypoint
                // which already advances the cursor).
            }
            assert!(
                resolved,
                "archer {} shooting-path scan exceeded sector {} point count",
                self.base.me, sector_idx
            );
        }
        false
    }

    // Final sprint: face shooting-point direction.

    fn attacking_archer_run_on_shooting_path_final_sprint(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(
                AiState::Attacking,
                Substate::AttackingArcherRunOnShootingPathTurn,
            );
            // FaceTo the shooting point's direction.
            // `my_shooting_point` carries the (sector, point) pair
            // set by `set_my_shooting_point`, so we can look up the
            // reserved point's direction directly.
            let (sec_idx, pt_idx) = self.my_shooting_point.unwrap_or_else(|| {
                panic!(
                    "archer {} final sprint has no reserved shooting point",
                    self.base.me
                )
            });
            let sector = global
                .archery_sectors
                .get(sec_idx as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "archer {} final sprint references missing archery sector {}",
                        self.base.me, sec_idx
                    )
                });
            let point = sector.points.get(pt_idx as usize).unwrap_or_else(|| {
                panic!(
                    "archer {} final sprint references missing point {} in sector {}",
                    self.base.me, pt_idx, sec_idx
                )
            });
            self.base.face_direction(point.direction, ctx);
        }
        false
    }

    // Done turning on shooting point: BattleDecisions if high, else
    // overview.

    fn attacking_archer_run_on_shooting_path_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            // If my elevation >= enemy's + 50 then set
            // enemy_seen_below and BattleDecisions; else
            // GetBattleOverview.
            let my_elevation = ctx.elevation as u16;
            if my_elevation >= self.enemy_had_this_elevation + 50 {
                self.enemy_seen_below = true;
                self.battle_decisions(sim, global, ctx, tick, grid);
            } else {
                self.get_battle_overview(0, ctx, tick);
            }
        }
        false
    }

    // Archer finished bending (reactiontime bend): in-trouble +
    // BattleDecisions.

    fn attacking_reactiontime_bending(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.i_am_in_trouble(self.base.primary_target);
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
        false
    }

    // Original returns an archer parked on either kind of shooting
    // path to ordinary duty when the hold timer expires.

    fn attacking_archer_wait_on_archery_path(
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

    // Shared "timer → ReturnToDuty" for bow archers waiting on
    // archery/bend points.

    fn attacking_archer_wait_on_bend_point(
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

    // Dummy training behavior: rotate direction every EVENT_DONE.

    fn attacking_dummy_behaviour(&mut self, stimulus_type: StimulusType, ctx: &AiContext) -> bool {
        if stimulus_type == StimulusType::EventDone {
            let new_dir = (ctx.direction + 3) & 15;
            self.base.face_direction(new_dir, ctx);
        }
        false
    }

    // Guarding a PC in coma: if still close & in coma keep
    // watching; else give up.

    fn attacking_swordfight_step_back(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
            self.base.launch_timer(20, ctx.frame);
        }
        false
    }

    // Officer approached brawl victim: kick off wake-up sequence.

    fn attacking_return_to_other_pc_after_menacing(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.begin_swordfight(ctx, tick);
        }
        false
    }

    // Running to enemy on a ladder: reach → face + focus + wait;
    // timer → reconsider.

    fn attacking_running_to_ladder(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        match stimulus_type {
            StimulusType::EventReachPoint => {
                if self.base.primary_target != 0 {
                    self.base.face_entity(self.base.primary_target, ctx);
                    self.base.outbox.actor.set_focus(self.base.primary_target);
                }
                self.set_state(AiState::Attacking, Substate::AttackingWaitingAtLadder);
                self.base.launch_timer(1, ctx.frame);
            }
            StimulusType::EventTimer => {
                self.reconsider_enemy_approach(false, 0.0, ctx, tick, grid);
            }
            _ => {}
        }
        false
    }

    // Waiting at ladder: if enemy still on lift, reface & rearm;
    // else reconsider.

    fn attacking_waiting_at_ladder(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // If primary target is on a lift sector, face+focus+wait;
            // else reconsider enemy approach.
            // This source arm calls
            // `mpPrimaryTarget->GetSector()->IsLift()` directly. It
            // must inspect the target element's physical sector, not
            // `RHArtificialIntelligence::Position(actor)`'s committed
            // destination-side sector during a door pass.
            let target_on_lift = grid
                .and_then(|g| {
                    tick.primary_target_live_position
                        .and_then(|position| position.sector)
                        .map(|s| g.sector_type(u32::from(s)).is_lift())
                })
                .unwrap_or(false);
            if target_on_lift {
                self.base.face_entity(self.base.primary_target, ctx);
                self.base.outbox.actor.set_focus(self.base.primary_target);
                self.base.launch_timer(20, ctx.frame);
            } else {
                self.reconsider_enemy_approach(false, 0.0, ctx, tick, grid);
            }
        }
        false
    }

    // Avenger on roof: reached pos, face seek & wait.

    fn attacking_run_to_avenger_on_roof(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventReachPoint {
            self.base
                .face_position_3d_with_ctx(self.base.seek_position, ctx);
            self.set_state(AiState::Attacking, Substate::AttackingWaitForAvengerOnRoof);
            self.base.launch_timer(100, ctx.frame);
        }
        false
    }

    // Wait for avenger: either re-face if detected, or SeekArea on
    // lost sight.

    fn attacking_wait_for_avenger_on_roof(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            // IsDetecting180Degrees(primary_target)? re-face +
            // 30-tick timer; else SeekArea(Position(mpMe),
            // AI_LOST_ENEMY_SEEK_RADIUS, 0).
            if self.base.primary_target != 0
                && self.is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
            {
                self.base.face_entity(self.base.primary_target, ctx);
                self.base.launch_timer(30, ctx.frame);
            } else {
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
            }
        }
        false
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
            Substate::MenacingPcInComa => {
                if stimulus_type == StimulusType::EventTimer {
                    // Assert IsPC + check MaxNormDistance < 100 &&
                    // IsUnconscious() && IsInComa(). `is_pc` and `in_coma`
                    // both live on `AiEntityView`, so the full triplet is
                    // checkable here without approximation.
                    let keep_watching = {
                        let v = ctx
                            .expect_entity_view(self.base.primary_target, "menacing-coma target");
                        let dx = (v.position.x - ctx.position.x).abs();
                        let dy = (v.position.y - ctx.position.y).abs();
                        v.is_pc && v.is_unconscious && v.in_coma && dx.max(dy) < 100.0
                    };
                    if keep_watching {
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    }
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
                                    .find(|d| d.door_index.0 == door_idx)
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
                    if self.base.primary_target != 0
                        && self
                            .is_detecting_180_degrees(self.base.primary_target as HumanHandle, ctx)
                    {
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
mod tests {
    use super::*;

    fn soldier_view_with_substate(
        handle: u32,
        substate: Substate,
    ) -> crate::ai_entity_view::AiEntityView {
        let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        });
        let mut view = crate::ai_entity_view::entity_view_from_entity(
            &entity,
            handle,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        );
        view.ai_state = AiState::Seeking;
        view.ai_substate = substate;
        view
    }

    #[test]
    fn officer_wait_for_instructed_group_keeps_full_original_seek_area_set() {
        let sim = crate::sim_rng::test_context();
        for member_substate in [
            Substate::SeekingSeekpoint,
            Substate::SeekingSeekpointWatching,
            Substate::SeekingSeekpointWatchingSidewards,
            Substate::SeekingSeekpointPassedAmbushPointLeft,
            Substate::SeekingSeekpointPassedAmbushPointRight,
            Substate::SeekingSeekpointCheckingAmbushPoint,
            Substate::SeekingSeekpointApproachingBeggar,
            Substate::SeekingSeekpointIdentifyingBeggar1,
            Substate::SeekingSeekpointIdentifyingBeggar2,
            Substate::SeekingNet,
        ] {
            let mut ai = EnemyAi::new(147);
            ai.base.current_state = AiState::Seeking;
            ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
            ai.alerted_us = vec![148];

            let mut views = crate::ai_entity_view::AiEntityViewMap::new();
            views.insert(148, soldier_view_with_substate(148, member_substate));
            let ctx = AiContext {
                frame: 7_915,
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            };

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventTimer),
                &mut AiGlobalState::default(),
                &ctx,
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.alerted_us, vec![148], "{member_substate:?}");
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingOfficerWaitForInstructedGroup,
                "{member_substate:?}"
            );
            assert!(ai.base.timer_is_running, "{member_substate:?}");
            assert_eq!(ai.base.when_does_timer_ring, 7_945, "{member_substate:?}");
            assert!(
                ai.base.outbox.reentrant.owner_work.is_empty(),
                "{member_substate:?}"
            );
        }
    }

    #[test]
    fn avenger_roof_timeout_seeks_from_live_owner_position() {
        // Original calls SeekArea(Position(mpMe), ...), not with the cached
        // position used to reach the avenger. Keep those positions far apart
        // so using the stale center cannot accidentally select these points.
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(149);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingWaitForAvengerOnRoof;
        ai.base.primary_target = 0;
        ai.base.seek_position = Position {
            x: 241.0,
            y: 862.0,
            ..Position::default()
        };

        let live_position = Position {
            x: 1_800.0,
            y: 2_200.0,
            ..Position::default()
        };
        let mut global = AiGlobalState::default();
        global.seek_points = [(1_810.0, 2_200.0), (1_820.0, 2_200.0), (1_830.0, 2_200.0)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| SeekPoint {
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
            frame: 12_000,
            position: live_position,
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.seek_center, live_position);
        assert_ne!(ai.seek_center, ai.base.seek_position);
        assert!(
            ai.my_seek_points.iter().any(|&id| id < 3),
            "live-center seek must select one of the nearby authored points"
        );
    }

    #[test]
    fn officer_wait_for_instructed_group_prunes_taking_net() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(147);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
        ai.alerted_us = vec![148];

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(
            148,
            soldier_view_with_substate(148, Substate::SeekingTakingNet),
        );
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(ai.alerted_us.is_empty());
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
        ));
    }

    #[test]
    fn officer_wait_for_instructed_group_waits_for_approaching_charly() {
        let sim = crate::sim_rng::test_context();
        for (charly_substate, should_wait) in [
            (Substate::SeekingCharlySentToOfficer, true),
            (Substate::SeekingCharlyGoToOfficer, true),
            (Substate::DefaultOnPost, false),
        ] {
            let mut ai = EnemyAi::new(147);
            ai.base.current_state = AiState::Seeking;
            ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
            ai.base.my_reconnaissance_report.report_type = ReportType::MissedCharly;
            ai.base.my_reconnaissance_report.charly = 148;

            let mut views = crate::ai_entity_view::AiEntityViewMap::new();
            views.insert(148, soldier_view_with_substate(148, charly_substate));
            let ctx = AiContext {
                frame: 7_915,
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            };

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventTimer),
                &mut AiGlobalState::default(),
                &ctx,
                &AiPerTickData::stub(),
                None,
            );

            if should_wait {
                assert!(ai.base.timer_is_running);
                assert_eq!(ai.base.when_does_timer_ring, 7_945);
                assert!(ai.base.outbox.reentrant.owner_work.is_empty());
            } else {
                assert!(matches!(
                    ai.base.outbox.reentrant.owner_work.as_slice(),
                    [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
                ));
            }
        }
    }

    #[test]
    fn archer_waiting_on_shooting_path_returns_to_duty_only_on_timer() {
        let sim = crate::sim_rng::test_context();
        for substate in [
            Substate::AttackingArcherWaitOnArcheryPath,
            Substate::AttackingArcherWaitOnArcheryPathBending,
        ] {
            let mut ai = EnemyAi::new(84);
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = substate;

            ai.think_expected_attacking_event(
                &sim,
                &Stimulus::new(StimulusType::EventDone),
                &mut AiGlobalState::default(),
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.base.current_state, AiState::Attacking);
            assert_eq!(ai.base.current_substate, substate);
            assert!(ai.base.outbox.reentrant.owner_work.is_empty());

            ai.think_expected_attacking_event(
                &sim,
                &Stimulus::new(StimulusType::EventTimer),
                &mut AiGlobalState::default(),
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert!(matches!(
                ai.base.outbox.reentrant.owner_work.as_slice(),
                [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
            ));
        }
    }

    #[test]
    fn fleeing_hiding_timer_invokes_enemy_return_to_duty() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(158);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingHiding;

        let handled = ai.think_expected_fleeing_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert!(handled);
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
        ));
    }

    #[test]
    fn approaching_new_enemy_close_gate_stretches_world_y() {
        // Task 274 frontier: the raw map-space delta fits inside the authored
        // 65+10 range, but Original SquareDistance stretches Y by the inverse
        // aspect ratio and therefore takes the re-approach arm.
        let owner = Position {
            x: 726.946_96,
            y: 2116.774,
            ..Position::default()
        };
        let target = Position {
            x: 710.678_9,
            y: 2049.651_1,
            ..Position::default()
        };
        let dx = target.x - owner.x;
        let dy = target.y - owner.y;
        assert!(dx * dx + dy * dy < 75_u32.pow(2) as f32);
        assert!(!approaching_new_enemy_is_close_enough(
            &target, 0.0, &owner, 0.0, 65,
        ));
    }

    #[test]
    fn approaching_new_enemy_close_gate_accepts_flat_nearby_target() {
        let owner = Position {
            x: 100.0,
            y: 100.0,
            ..Position::default()
        };
        let target = Position {
            x: 110.0,
            y: 110.0,
            ..Position::default()
        };

        assert!(approaching_new_enemy_is_close_enough(
            &target, 0.0, &owner, 0.0, 65,
        ));
    }

    #[test]
    fn approaching_new_enemy_close_gate_uses_literal_positions() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(58);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
        ai.base.primary_target = 103;

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 58,
                sword_range_default: 65,
                ..crate::ai_enemy::FighterSnapshot::default()
            });
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 103,
                // `Position(target)` is a planning position and remains the
                // destination for the GoNear arm, but it is not GetPosition().
                position: Position {
                    x: 500.0,
                    y: 0.0,
                    ..Position::default()
                },
                elevation: 0.0,
                ..crate::ai_enemy::FighterSnapshot::default()
            });
        tick.owner_live_position = Some(Position {
            x: 100.0,
            y: 100.0,
            ..Position::default()
        });
        tick.primary_target_snapshot_handle = 103;
        tick.primary_target_live_position = Some(Position {
            x: 110.0,
            y: 110.0,
            ..Position::default()
        });
        let ctx = AiContext {
            // The owner's planning position is also deliberately far from
            // the target's planning position.
            position: Position::default(),
            elevation: 0.0,
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.base.outbox.actor.set_principal, Some(103));
    }

    #[test]
    #[should_panic(expected = "primary target 103 is missing its required fighter snapshot")]
    fn approaching_new_enemy_requires_primary_target_snapshot() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(58);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
        ai.base.primary_target = 103;

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "primary target 103 does not match tick snapshot handle 102")]
    fn approaching_new_enemy_rejects_stale_primary_target_geometry() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(58);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
        ai.base.primary_target = 103;

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry
            .push(crate::ai_enemy::FighterSnapshot {
                handle: 103,
                ..crate::ai_enemy::FighterSnapshot::default()
            });
        tick.primary_target_snapshot_handle = 102;

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &tick,
            None,
        );
    }

    #[test]
    fn bow_running_behind_shield_faces_target_with_its_elevation() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(74);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingBowRunningBehindShieldBearer;
        ai.base.primary_target = 170;

        let mut target = pc_view(crate::element::Posture::Upright);
        target.position = Position {
            x: 265.357_67,
            y: 1023.334_5,
            ..Position::default()
        };
        target.elevation = 151.123_84;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(170, target);
        let ctx = AiContext {
            position: Position {
                x: 436.932_5,
                y: 1227.554,
                ..Position::default()
            },
            elevation: 45.0,
            direction: 3,
            self_action_state: crate::element::ActionState::Moving,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        // A flat map-space face selects sector 15 at this boundary. The
        // Original element overload adds `(SWORD)151.12384 - 45 == 106`
        // to dy and therefore authors sector 14.
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(
                265.357_67 - 436.932_5,
                1023.334_5 - 1227.554
            ),
            15
        );

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        let [turn] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("shield-bearer arrival must author exactly one turn");
        };
        assert_eq!(turn.order_type, crate::order::OrderType::Turning);
        assert_eq!(turn.explicit_direction, Some(14));
    }

    #[test]
    fn officer_wait_missed_soldier_does_not_relaunch_timer() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedSoldier;
        ai.base.antagonist = 2;
        ai.missed_soldier_timer = 11;

        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
            handle: 2,
            active: true,
            position: Position::default(),
            position_world: crate::coordinates::WorldPoint3D::ZERO,
            direction: 0,
            rank: ProfileRank::Soldier,
            ai_state: AiState::Seeking,
            ai_substate: Substate::SeekingSeekpoint,
            is_able_to_fight: true,
            is_dead: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 1,
            detected_body: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 300,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        });

        let mut self_view = pc_view(crate::element::Posture::Upright);
        self_view.is_able_to_fight = true;
        self_view.active = true;
        let mut missed_view = pc_view(crate::element::Posture::Upright);
        missed_view.is_able_to_fight = false;
        missed_view.is_unconscious = true;
        missed_view.active = true;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(1, self_view);
        views.insert(2, missed_view);
        let ctx = AiContext {
            frame: 34_522,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.missed_soldier_timer, 12);
        assert!(!ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 0);
    }

    #[test]
    fn goto_post_arrival_runs_enemy_attentive_tail_after_turn_request() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(57);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultGotoPost;
        ai.base.initial_view_direction = 4;
        ai.attentive = true;
        ai.will_be_attentive = true;
        let ctx = AiContext {
            direction: 0,
            self_action_state: crate::element::ActionState::Waiting,
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPostTurn);
        let state_change = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(change) => Some(change),
                _ => None,
            })
            .expect("goto-post arrival must use EnemyAi::set_state");
        let turn_prefix = state_change
            .actor_effects_before_callback
            .as_ref()
            .expect("FaceTo must precede the virtual SetState call");
        assert_eq!(turn_prefix.orders.len(), 1);
        assert_eq!(
            turn_prefix.orders[0].order_type,
            crate::order::OrderType::Turning
        );
        let attentive = ai
            .base
            .outbox
            .actor
            .set_attentive_mode
            .expect("Default SetState must queue its attentive-mode tail");
        assert!(!attentive.target);
        assert!(!attentive.fast_officer_variant);
        assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Green);
    }

    #[test]
    fn reached_beggar_launches_one_ordered_turn_then_response_sequence() {
        use crate::element::{Command, EntityId, Posture};
        use crate::entity_id::SoldierId;
        use crate::sequence::{Field, FieldValue};

        for (archer, response, timer) in [
            (false, Command::StartMenace, 30),
            (true, Command::EquipBow, 100),
        ] {
            let sim = crate::sim_rng::test_context();
            let mut ai = EnemyAi::new(1);
            ai.base.owner_entity_id = Some(EntityId::Soldier(SoldierId(1)));
            ai.set_state(
                AiState::Seeking,
                Substate::SeekingSeekpointApproachingBeggar,
            );
            ai.beggar_to_examine = 17;
            ai.beggar_is_npc = false;
            ai.is_archer_unit = archer;

            let mut beggar = pc_view(Posture::SimulatingBeggar);
            beggar.position = Position {
                x: 140.0,
                y: 80.0,
                ..Position::default()
            };
            let mut views = crate::ai_entity_view::AiEntityViewMap::new();
            views.insert(17, beggar);
            let ctx = AiContext {
                frame: 400,
                position: Position {
                    x: 100.0,
                    y: 100.0,
                    ..Position::default()
                },
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            };

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventReachPoint),
                &mut AiGlobalState::default(),
                &ctx,
                &AiPerTickData::stub(),
                None,
            );

            assert!(ai.base.outbox.actor.orders.is_empty());
            assert!(ai.base.outbox.actor.launch_commands.is_empty());
            let [sequence] = ai.base.outbox.actor.launch_sequences.as_slice() else {
                panic!("beggar identification must launch exactly one sequence");
            };
            assert_eq!(sequence.elements.len(), 2);
            assert_eq!(sequence.elements[0].command, Command::TurnFast);
            assert_eq!(sequence.elements[0].command_level, 1);
            assert_eq!(sequence.elements[1].command, response);
            assert_eq!(sequence.elements[1].command_level, 2);
            let expected_direction =
                crate::position_interface::vector_to_sector_0_to_15_iso(40.0, -20.0);
            assert!(matches!(
                sequence.elements[0].get_property(Field::Direction),
                Some(FieldValue::Integer(direction)) if *direction == expected_direction as u32
            ));
            assert_eq!(ai.base.when_does_timer_ring, 400 + timer);
        }
    }

    #[test]
    fn identified_npc_beggar_shows_face_then_identifies_himself() {
        use crate::ai::{AiOwnerWork, Remark};
        use crate::element::Command;

        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(
            AiState::Seeking,
            Substate::SeekingSeekpointIdentifyingBeggar1,
        );
        ai.base.outbox = crate::ai::AiOutbox::default();
        ai.beggar_to_examine = 70;
        ai.beggar_is_npc = true;

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        let prefix = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                AiOwnerWork::StateChange(change) => change.actor_effects_before_callback.as_ref(),
                _ => None,
            })
            .expect("beggar response must precede the identifying-2 SetState callback");
        assert_eq!(prefix.launch_on_target, vec![(70, Command::BeggarShowFace)]);
        assert_eq!(
            prefix.say_on_target,
            vec![(70, Remark::CivBeggarIdentifiesHimself)]
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingSeekpointIdentifyingBeggar2
        );
    }

    #[test]
    fn combat_alert_ignores_timer_until_reaching_the_alert_point() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingCombatAlert);

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, Substate::SeekingCombatAlert);
    }

    #[test]
    fn combat_alert_reachpoint_starts_lost_enemy_seek() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingCombatAlert);
        ai.base.seek_position = Position {
            x: 120.0,
            y: 240.0,
            ..Position::default()
        };
        let mut global = AiGlobalState::default();

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.seek_center, ai.base.seek_position);
        assert!(ai.seek_flags.is_empty());
        assert!(ai.personal_seek_point_2.is_some());
        assert_ne!(ai.base.current_substate, Substate::SeekingCombatAlert);
    }

    #[test]
    fn heardsteps_arrival_starts_zero_radius_walking_seek() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(138);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingHeardsteps;
        ai.base.seek_position = Position {
            x: 900.0,
            y: 700.0,
            ..Position::default()
        };
        let here = Position {
            x: 1630.6875,
            y: 1630.921875,
            ..Position::default()
        };
        let ctx = AiContext {
            frame: 1257,
            position: here,
            camp: crate::element::Camp::Lacklandists,
            self_animation: crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            ..AiContext::default()
        };
        let mut global = AiGlobalState::default();

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventReachPoint),
                &mut global,
                &ctx,
                &AiPerTickData::stub(),
                None,
            );
        });

        assert_eq!(
            draws,
            vec![
                crate::sim_rng::RngSite::SeekPointDirectionPattern,
                crate::sim_rng::RngSite::SeekPointAcceptance,
            ]
        );
        assert_eq!(ai.seek_center, here);
        assert_eq!(
            ai.seek_flags,
            SeekFlags::LOCATION_FIRST | SeekFlags::WALKING
        );
        assert_eq!(ai.base.current_substate, Substate::SeekingSeekpoint);
        assert_eq!(ai.actual_seek_point, Some(1111));
        assert_eq!(
            ai.base.seek_position,
            Position {
                x: 900.0,
                y: 700.0,
                ..Position::default()
            },
            "selecting a route point must preserve the semantic heard-steps position"
        );
        assert!(
            ai.personal_seek_point_1
                .as_ref()
                .is_some_and(|point| point.locked)
        );
        // The old shortcut authored a Face/Turn order here. SeekArea owns the
        // post-arrival lifecycle instead; at this pure-AI boundary it has no
        // live actor order (the zero-distance completion is settled by the
        // enclosing Think lifecycle).
        assert!(ai.base.outbox.actor.orders.is_empty());
    }

    #[test]
    fn reaching_near_officer_redispatches_reachpoint_synchronously() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingRunningToOfficer);
        ai.base.antagonist = 2;
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
            handle: 2,
            active: true,
            position: Position::default(),
            position_world: crate::coordinates::WorldPoint3D::ZERO,
            direction: 0,
            rank: ProfileRank::Officer,
            ai_state: AiState::Default,
            ai_substate: Substate::DefaultOnPost,
            is_able_to_fight: true,
            is_dead: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 0,
            detected_body: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 400,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        });

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &tick,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingRunningToOfficerSeen
        );
        assert_eq!(
            ai.base.outbox.reentrant.self_stimuli,
            vec![StimulusType::EventReachPoint]
        );
        assert!(!ai.base.timer_is_running);
    }

    #[test]
    fn parade_timer_stops_only_an_active_normal_parry() {
        let sim = crate::sim_rng::test_context();

        for (action_state, should_stop) in [
            (crate::element::ActionState::WaitingSword, false),
            (crate::element::ActionState::ParryingSword, true),
            (crate::element::ActionState::ParryingSwordLow, false),
        ] {
            let mut ai = EnemyAi::new(1);
            ai.set_state(AiState::Attacking, Substate::AttackingSwordfightParade);
            let ctx = AiContext {
                frame: 325,
                self_action_state: action_state,
                ..AiContext::default()
            };

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::EventTimer),
                &mut AiGlobalState::default(),
                &ctx,
                &AiPerTickData::stub(),
                None,
            );

            // The stop-parry command is issued before the handler's SetState
            // suspends the actor-outbox prefix into the queued state-change
            // owner work; collect commands from both places.
            let mut launch_commands: Vec<crate::element::Command> = Vec::new();
            for work in &ai.base.outbox.reentrant.owner_work {
                if let crate::ai::AiOwnerWork::StateChange(notification) = work
                    && let Some(effects) = &notification.actor_effects_before_callback
                {
                    launch_commands.extend(effects.launch_commands.iter().copied());
                }
            }
            launch_commands.extend(ai.base.outbox.actor.launch_commands.iter().copied());
            assert_eq!(
                launch_commands == vec![crate::element::Command::StopParrySword],
                should_stop,
                "unexpected stop-parry emission for {action_state:?}"
            );
            assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
            assert_eq!(ai.base.when_does_timer_ring, 345);
        }
    }

    fn pc_view(posture: crate::element::Posture) -> crate::ai_entity_view::AiEntityView {
        let entity = crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        crate::ai_entity_view::entity_view_from_entity(
            &entity,
            41,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        )
    }

    fn run_reactiontime_turning(
        frame: u32,
        owner_forecast: Position,
        owner_live: Position,
        target_forecast: Position,
        target_live: Position,
    ) -> EnemyAi {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(155);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontimeTurning;
        ai.base.primary_target = 345;

        let mut target = pc_view(crate::element::Posture::Upright);
        target.position = target_forecast;
        target.detection_position =
            crate::coordinates::MapPoint::new(target_forecast.x, target_forecast.y);
        target.detection_position_world =
            crate::coordinates::WorldPoint3D::new(target_live.x, target_live.y + 480.0, 480.0);
        target.current_animation = crate::order::OrderType::WaitingUpright;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(345, target);
        let ctx = AiContext {
            frame,
            position: owner_forecast,
            self_body_position_world: crate::coordinates::WorldPoint3D::new(
                owner_live.x,
                owner_live.y + 480.0,
                480.0,
            ),
            elevation: 480.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_attacking_event(
            &sim,
            &Stimulus::new(StimulusType::EventDone),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );
        ai
    }

    #[test]
    fn reactiontime_turning_uses_live_distance_during_door_pass() {
        let owner_forecast = Position {
            x: 367.0,
            y: 757.0,
            ..Position::default()
        };
        let target_forecast = Position {
            x: 360.0,
            y: 750.0,
            ..Position::default()
        };
        let owner_live = owner_forecast;
        let target_live = Position {
            x: 354.0,
            y: 731.0,
            ..Position::default()
        };

        let forecast_max_norm = (target_forecast.x - owner_forecast.x)
            .abs()
            .max((target_forecast.y - owner_forecast.y).abs());
        let live_distance = ai_square_distance(&target_live, 480.0, &owner_live, 480.0).sqrt();
        assert!(forecast_max_norm < 30.0);
        assert!((47.0..48.0).contains(&live_distance));

        let ai = run_reactiontime_turning(
            36_150,
            owner_forecast,
            owner_live,
            target_forecast,
            target_live,
        );

        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(
            ai.base.when_does_timer_ring,
            36_150 + parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32
        );
    }

    #[test]
    fn reactiontime_turning_uses_one_tick_timer_when_live_target_is_close() {
        let owner_live = Position {
            x: 367.0,
            y: 757.0,
            ..Position::default()
        };
        let target_live = Position {
            x: 380.0,
            y: 757.0,
            ..Position::default()
        };
        let owner_forecast = owner_live;
        let target_forecast = Position {
            x: 500.0,
            y: 900.0,
            ..Position::default()
        };

        let forecast_max_norm = (target_forecast.x - owner_forecast.x)
            .abs()
            .max((target_forecast.y - owner_forecast.y).abs());
        let live_distance = ai_square_distance(&target_live, 480.0, &owner_live, 480.0).sqrt();
        assert!(forecast_max_norm > 30.0);
        assert_eq!(live_distance, 13.0);

        let ai = run_reactiontime_turning(
            36_151,
            owner_forecast,
            owner_live,
            target_forecast,
            target_live,
        );

        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(ai.base.when_does_timer_ring, 36_152);
    }

    #[test]
    fn instructed_soldier_adds_officers_selected_body_after_speech() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(89);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSoldierGetInstructedByOfficer;
        ai.base.antagonist = 91;

        let alert_point = Position {
            x: 2100.0,
            y: 1600.0,
            ..Position::default()
        };
        let officer_position = Position {
            x: 2050.0,
            y: 1550.0,
            ..Position::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
            handle: 91,
            active: true,
            position: officer_position,
            position_world: crate::coordinates::WorldPoint3D::ZERO,
            direction: 0,
            rank: ProfileRank::Officer,
            ai_state: AiState::Seeking,
            ai_substate: Substate::SeekingOfficerWaitForInstructedSoldier,
            is_able_to_fight: true,
            is_dead: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: alert_point,
            patrol_chief: None,
            antagonist: 89,
            detected_body: 97,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 400,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        });

        let mut body = pc_view(crate::element::Posture::Tied);
        body.kind = crate::ai_entity_view::EntityKind::Soldier;
        body.is_pc = false;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(97, body);
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        // Speech completion carries no body payload. Original reads the
        // officer's selected body directly before sending CALL_YOURTALK_2.
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventMyTalk2),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.first(),
            Some(CrossNpcAction::SendStimulus {
                target: 91,
                stimulus_type: StimulusType::CallYourTalk2,
                info: StimulusInfo::None,
                ..
            })
        ));
        let mut added_detectables = ai.base.outbox.actor.add_detectables.clone();
        for work in &ai.base.outbox.reentrant.owner_work {
            match work {
                AiOwnerWork::ActorEffects(effects) => {
                    added_detectables.extend(effects.add_detectables.iter().copied());
                }
                AiOwnerWork::StateChange(change) => {
                    if let Some(effects) = &change.actor_effects_before_callback {
                        added_detectables.extend(effects.add_detectables.iter().copied());
                    }
                }
                _ => {}
            }
        }
        assert_eq!(
            added_detectables,
            vec![(
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(97)),
                crate::element::DetectableType::Body,
            )]
        );
        assert_eq!(ai.base.alert_soldiers_point, alert_point);
        assert_eq!(ai.officers_position, officer_position);
    }

    #[test]
    fn seeking_body_reach_rejects_a_body_outside_the_live_sixty_unit_gate() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(61);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.base.detected_body = 170;

        let mut owner = pc_view(crate::element::Posture::Upright);
        owner.detection_position_world =
            crate::coordinates::WorldPoint3D::new(413.38, 1850.28, 150.0);
        let mut body = pc_view(crate::element::Posture::Dead);
        body.is_able_to_fight = false;
        body.is_dead = true;
        body.detection_position_world = crate::coordinates::WorldPoint3D::new(737.20, 1869.22, 0.0);

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(61, owner);
        views.insert(170, body);
        let ctx = AiContext {
            position: Position {
                x: 413.38,
                y: 1700.28,
                ..Position::default()
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_ne!(
            ai.base.current_substate,
            Substate::SeekingBodyLookingDeadBody
        );
        let mut added_detectables = ai.base.outbox.actor.add_detectables.clone();
        for work in &ai.base.outbox.reentrant.owner_work {
            match work {
                AiOwnerWork::ActorEffects(effects) => {
                    added_detectables.extend(effects.add_detectables.iter().copied());
                }
                AiOwnerWork::StateChange(change) => {
                    if let Some(effects) = &change.actor_effects_before_callback {
                        added_detectables.extend(effects.add_detectables.iter().copied());
                    }
                }
                _ => {}
            }
        }
        assert_eq!(
            added_detectables,
            vec![(
                crate::element::EntityId::Pc(crate::entity_id::PcId(170)),
                crate::element::DetectableType::Body,
            )]
        );
    }

    #[test]
    fn seeking_body_reach_does_not_turn_toward_a_nearby_dead_body() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(61);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.base.detected_body = 170;
        ai.base.seek_position = Position {
            x: 120.0,
            y: 100.0,
            ..Position::default()
        };

        let mut owner = pc_view(crate::element::Posture::Upright);
        owner.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 250.0, 150.0);
        let mut body = pc_view(crate::element::Posture::Dead);
        body.is_able_to_fight = false;
        body.is_dead = true;
        body.detection_position_world = crate::coordinates::WorldPoint3D::new(120.0, 250.0, 150.0);

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(61, owner);
        views.insert(170, body);
        let ctx = AiContext {
            direction: 9,
            position: Position {
                x: 100.0,
                y: 100.0,
                ..Position::default()
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingBodyLookingDeadBody
        );
        assert!(ai.already_seen_bodies.contains(&170));
        assert!(ai.base.outbox.actor.launch_commands.is_empty());
        assert!(ai.base.outbox.actor.launch_sequences.is_empty());
    }

    #[test]
    fn seeking_body_reach_returns_to_duty_when_the_nearby_body_recovered() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(61);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingBody;
        ai.base.detected_body = 63;

        let mut owner = pc_view(crate::element::Posture::Upright);
        owner.detection_position_world =
            crate::coordinates::WorldPoint3D::new(413.38, 1850.2786, 150.001);
        let mut recovered_body = pc_view(crate::element::Posture::Upright);
        recovered_body.is_dead = false;
        recovered_body.is_unconscious = false;
        recovered_body.detection_position_world =
            crate::coordinates::WorldPoint3D::new(410.0648, 1850.421, 150.001);

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(61, owner);
        views.insert(63, recovered_body);
        let ctx = AiContext {
            position: Position {
                x: 413.38,
                y: 1700.2776,
                ..Position::default()
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_ne!(
            ai.base.current_substate,
            Substate::SeekingBodyLookingDeadBody
        );
        assert!(ai.already_seen_bodies.is_empty());
        assert!(
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(work, AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }))
        );
    }

    #[test]
    fn arrow_watching_ignores_event_done() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        // Original: RHArtificialMalignity::ThinkExpectedEvent handles only
        // EVENT_TIMER and EVENT_MYTALK_1 for the just-watching substate.
        for substate in [
            Substate::SeekingArrowJustWatching,
            Substate::SeekingArrowJustWatchingSidewards,
        ] {
            let mut ai = EnemyAi::new(1);
            ai.set_state(AiState::Seeking, substate);
            let mut global = AiGlobalState::default();

            ai.think_expected_event(
                sim,
                &Stimulus::new(StimulusType::EventDone),
                &mut global,
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.base.current_state, AiState::Seeking);
            assert_eq!(ai.base.current_substate, substate);
        }
    }

    #[test]
    fn bow_transition_states_ignore_shield_bearer_coordinate_calls() {
        let sim = crate::sim_rng::test_context();

        // RHArtificialMalignity::ThinkExpectedAttackingEvent only handles
        // CALL_COORDINATE while the archer is in BOW_SHOOTING.  A shield
        // bearer can still make the synchronous call while its archer is
        // loading or aiming; these substates deliberately ignore it.
        for substate in [
            Substate::AttackingBowObservingLoading,
            Substate::AttackingBowLoading,
            Substate::AttackingBowAiming,
        ] {
            let mut ai = EnemyAi::new(1);
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = substate;
            ai.base.timer_is_running = true;
            ai.base.when_does_timer_ring = 777;

            ai.think_expected_event(
                &sim,
                &Stimulus::new(StimulusType::CallCoordinate),
                &mut AiGlobalState::default(),
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            );

            assert_eq!(ai.base.current_state, AiState::Attacking);
            assert_eq!(ai.base.current_substate, substate);
            assert!(ai.base.timer_is_running);
            assert_eq!(ai.base.when_does_timer_ring, 777);
            assert!(ai.base.outbox.actor.orders.is_empty());
            assert!(ai.base.outbox.actor.launch_sequences.is_empty());
        }
    }

    #[test]
    fn goto_chief_reach_faces_live_chief_with_elevation() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(53);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultGotoChief;
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(47),
        ));
        let chief = crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        });
        let mut chief_view = crate::ai_entity_view::entity_view_from_entity(
            &chief,
            47,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        );
        chief_view.position = Position {
            x: 1033.585_9,
            y: 2036.767,
            ..Position::default()
        };
        chief_view.elevation = 25.100_779;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(47, chief_view);
        let ctx = AiContext {
            frame: 34_866,
            position: Position {
                x: 1021.08,
                y: 2031.790_4,
                ..Position::default()
            },
            elevation: 27.711_25,
            direction: 6,
            self_action_state: crate::element::ActionState::Waiting,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.patrol_chief_position = ctx.entity_position(47).expect("chief view");

        // The old cached-position overload selects the current sector and
        // incorrectly completes synchronously. The Original entity overload
        // adds `(SWORD)25.100779 - 27.71125` to Y and selects sector 5.
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(
                tick.patrol_chief_position.x - ctx.position.x,
                tick.patrol_chief_position.y - ctx.position.y,
            ),
            6
        );

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert!(!ai.base.already_turned);
        let [crate::ai::AiOwnerWork::StateChange(notification)] =
            ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!("goto-chief arrival must stage exactly one state change");
        };
        let effects = notification
            .actor_effects_before_callback
            .as_ref()
            .expect("Face must precede SetState");
        let [turn] = effects.orders.as_slice() else {
            panic!("elevation-aware chief facing must author exactly one turn");
        };
        assert_eq!(turn.order_type, crate::order::OrderType::Turning);
        assert_eq!(turn.explicit_direction, Some(5));
        assert_eq!(
            ai.base.current_substate,
            Substate::DefaultPatrolEnrouteWaiting
        );
        assert_eq!(ai.base.when_does_timer_ring, 35_066);
    }

    #[test]
    #[should_panic(expected = "running on shooting path has no archery sector")]
    fn shooting_path_does_not_fabricate_an_end_of_path_recovery() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(
            AiState::Attacking,
            Substate::AttackingArcherRunOnShootingPath,
        );
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "final sprint has no reserved shooting point")]
    fn shooting_path_final_sprint_requires_its_reserved_point() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(
            AiState::Attacking,
            Substate::AttackingArcherRunOnShootingPathFinalSprint,
        );
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "receiving CALL_REPORT requires civilian 42 view")]
    fn civilian_report_does_not_fabricate_enemy_data_when_sender_is_missing() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingWaitForAlertingCivilian);
        let mut stimulus = Stimulus::new(StimulusType::CallReport);
        stimulus.info = StimulusInfo::Hint(Hint {
            seek_point: Position::default(),
            seek_flags: 0,
            who_tells_me: 42,
        });
        ai.think_expected_event(
            &sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );
    }
}
