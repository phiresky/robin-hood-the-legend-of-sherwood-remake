//! Expected-event handlers for the `Attacking` state family.
//!
//! This mirrors the `STATE_ATTACKING` section of
//! `RHArtificialMalignity::ThinkExpectedEvent` in the Original.

use super::*;

impl EnemyAi {
    pub(super) fn think_expected_attacking_event(
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
        let debug_decision_path = super::super::decision_path_debug_enabled()
            && super::super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} co={:?} stage=reactiontime_running_event stimulus={stimulus_type:?} state={:?}/{:?} primary={} rider={} couldnt={} already={} owner_work_before={:?}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                self.base.current_state,
                self.base.current_substate,
                self.base.primary_target,
                ctx.self_is_rider,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
            );
        }
        if stimulus_type == StimulusType::EventTimer
            || stimulus_type == StimulusType::EventReachPoint
        {
            self.base.stop_all();
            self.i_am_in_trouble(self.base.primary_target);
            self.battle_decisions(sim, global, ctx, tick, grid);
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=reactiontime_running_done state={:?}/{:?} primary={} couldnt={} already={} owner_work_after={:?}",
                    ctx.frame,
                    self.base.me,
                    self.base.current_state,
                    self.base.current_substate,
                    self.base.primary_target,
                    self.base.couldnt_reachpoint,
                    self.base.already_on_point,
                    self.base.outbox.reentrant.owner_work,
                );
            }
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
            if crate::ai_enemy::combat_positions::reconsider_position_debug_matches(
                || ctx.frame,
                || ctx.original_creation_order,
                || self.base.me,
            ) {
                eprintln!(
                    "[RECONSIDER_STIMULUS] frame={} owner={} creation_order={:?} stimulus={stimulus_type:?}",
                    ctx.frame, self.base.me, ctx.original_creation_order,
                );
            }
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
                // `sword_range` is persistent profile state; unlike action
                // state or positions, it does not become stale when the
                // per-tick fighter registry omits the owner.
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
                if super::super::them_lifecycle_debug_matches(ctx) {
                    eprintln!(
                        "[THEM frame={} co={:?} me={} phase=timer_entry state={:?} substate={:?} route=bow_behind_shield]",
                        ctx.frame,
                        ctx.original_creation_order,
                        self.base.me,
                        self.base.current_state,
                        self.base.current_substate,
                    );
                }
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

    pub(super) fn attacking_door_fight_waiting(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        stimulus_type: StimulusType,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            if super::super::them_lifecycle_debug_matches(ctx) {
                eprintln!(
                    "[THEM frame={} co={:?} me={} phase=timer_entry state={:?} substate={:?} route=officer_orders_waiting]",
                    ctx.frame,
                    ctx.original_creation_order,
                    self.base.me,
                    self.base.current_state,
                    self.base.current_substate,
                );
            }
            // Original resumes a door fight through GetBattleOverview: it
            // rebuilds the enemy list and starts the left/right observation
            // sequence. Going directly to BattleDecisions skips those
            // observation states and can synchronously enter SeekArea,
            // consuming selection RNG which Original has not requested yet.
            self.get_battle_overview(0, ctx, tick);
        }
        false
    }

    // ============ PHALANX / SHIELD-BEARER ============

    pub(super) fn attacking_protecting_with_shield(
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
                .unwrap_or_else(|| {
                    panic!(
                        "shield bearer {} is missing its required fighter snapshot",
                        self.base.me
                    )
                });
            if crate::ai_enemy::battle_decision_debug_enabled() {
                eprintln!(
                    "SHIELD_TIMER frame={} me={} action={:?} shield={} left={} right={} archer_behind={} target={} target_action={:?}",
                    ctx.frame,
                    self.base.me,
                    my_action,
                    my_action.is_shield(),
                    self.left_combat_neighbour,
                    self.right_combat_neighbour,
                    self.archer_behind_me,
                    self.base.primary_target,
                    self.find_fighter(self.base.primary_target, tick)
                        .map(|f| f.action_state),
                );
            }

            if !my_action.is_shield() {
                // Reestablish shield state
                let (target_pos, target_elevation) = self
                    .find_fighter(self.base.primary_target, tick)
                    // Original stores `mpPrimaryTarget->GetPosition()` in
                    // RHFIELD_SHIELD_DANGER_POINT. This is the raw element
                    // position, not the AI `Position()` helper that projects
                    // actors passing a door onto the gate endpoint.
                    .map(|f| (f.raw_position, f.elevation as f32))
                    .unwrap_or_else(|| {
                        panic!(
                            "shield bearer {} requires primary target {} in the fighter registry",
                            self.base.me, self.base.primary_target
                        )
                    });
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
                if self.base.primary_target == 0 {
                    self.base.primary_target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                }
                if self.base.primary_target == 0 {
                    tracing::error!(
                        soldier = self.base.me,
                        "shield bearer protecting an archer has no primary target"
                    );
                    self.get_battle_overview(0, ctx, tick);
                    return false;
                }
                let target_pos = self
                    .find_fighter(self.base.primary_target, tick)
                    .map(|f| f.position)
                    .unwrap_or_else(|| {
                        panic!(
                            "shield bearer {} requires primary target {} in the fighter registry",
                            self.base.me, self.base.primary_target
                        )
                    });
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
                        .unwrap_or_else(|| {
                            panic!(
                                "shield bearer {} requires target {} in the fighter registry",
                                self.base.me, target
                            )
                        });
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
                    .unwrap_or_else(|| {
                        panic!(
                            "advancing shield bearer {} requires primary target {} in the fighter registry",
                            self.base.me, self.base.primary_target
                        )
                    });
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
                        // RHFIELD_SHIELD_DANGER_POINT stores the target
                        // element's raw GetPosition().  The AI-facing
                        // Position() may instead project an actor traversing
                        // a door onto its gate endpoint.
                        .map(|f| (f.raw_position, f.elevation as f32))
                        .unwrap_or_else(|| {
                            panic!(
                                "phalanx runner {} requires target {} in the fighter registry",
                                self.base.me, target
                            )
                        });
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

    pub(super) fn attacking_phalanx(
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
                    .unwrap_or_else(|| {
                        panic!(
                            "phalanx soldier {} is missing its required fighter snapshot",
                            self.base.me
                        )
                    });

                tracing::trace!(
                    target: "robin_engine::ai_enemy::phalanx",
                    me = self.base.me,
                    frame = ctx.frame,
                    ?my_action,
                    primary = self.base.primary_target,
                    "phalanx timer"
                );
                let phalanx_debug = crate::ai_enemy::battle_decision_debug_enabled();
                if phalanx_debug {
                    eprintln!(
                        "PHALANX_TIMER frame={} me={} action={:?} left={} right={} target={} archer_behind={}",
                        ctx.frame,
                        self.base.me,
                        my_action,
                        self.left_combat_neighbour,
                        self.right_combat_neighbour,
                        self.base.primary_target,
                        self.archer_behind_me
                    );
                }
                if !my_action.is_shield() && self.base.primary_target != 0 {
                    // Reestablish shield state
                    let (target_pos, target_elevation) = self
                        .find_fighter(self.base.primary_target, tick)
                        .map(|f| (f.raw_position, f.elevation as f32))
                        .unwrap_or_else(|| {
                            panic!(
                                "phalanx soldier {} requires primary target {} in the fighter registry",
                                self.base.me, self.base.primary_target
                            )
                        });
                    self.base.raise_shield(target_pos, target_elevation);
                    self.base.launch_timer(20, ctx.frame);
                } else if !self.reconsider_phalanx(sim, global, ctx, tick, grid) {
                    if self.base.primary_target != 0 {
                        // No phalanx correction — maybe correct direction
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or_else(|| {
                                panic!(
                                    "phalanx soldier {} requires primary target {} in the fighter registry",
                                    self.base.me, self.base.primary_target
                                )
                            });
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
                if phalanx_debug {
                    eprintln!(
                        "PHALANX_TIMER_EXIT frame={} me={} substate={:?}",
                        ctx.frame, self.base.me, self.base.current_substate
                    );
                }
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
                        .unwrap_or_else(|| {
                            panic!(
                                "phalanx soldier {} requires protected archer {} in the fighter registry",
                                self.base.me, self.archer_behind_me
                            )
                        });
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
                        // VIP variant — a bare sword draw, NOT an engagement.
                        // `RHartificialmalignity.cpp:4433-4438` builds the
                        // ENTER_SWORDFIGHT element with
                        // `RHFIELD_OPPONENT = 0` and
                        // `RHFIELD_JUMPLINE_DESTINATION = 0`, so
                        // `RHElementActorHuman::Translate(ENTER_SWORDFIGHT)`
                        // (RHelementactorhuman.cpp:1319-1404) skips
                        // `EnterSwordFight` and stores a NULL
                        // `pOrder->pAntagonist` on the raise-sword order. The
                        // soldier's RAISING_SWORD arm
                        // (RHelementactorsoldier.cpp:1250-1259) then performs
                        // neither the `SetDirection` nor the `Turn`: the VIP
                        // draws his sword without turning toward the comatose
                        // PC. Passing the target here made Rust face him.
                        self.base.outbox.actor.enter_swordfight =
                            Some(EnterSwordfightRequest::RaiseSword);
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
                    if tick.primary_target_snapshot_handle != self.base.primary_target {
                        panic!(
                            "ApproachingSleepingEnemy primary target {} does not match tick snapshot handle {}",
                            self.base.primary_target, tick.primary_target_snapshot_handle
                        );
                    }
                    let target_live_position =
                        tick.primary_target_live_position.unwrap_or_else(|| {
                            panic!(
                                "ApproachingSleepingEnemy primary target {} is missing its literal position",
                                self.base.primary_target
                            )
                        });
                    let owner_live_position = tick.owner_live_position.unwrap_or_else(|| {
                        panic!(
                            "ApproachingSleepingEnemy owner {} is missing its literal position",
                            self.base.me
                        )
                    });
                    if ai_square_distance(
                        &target_live_position,
                        view.elevation,
                        &owner_live_position,
                        ctx.elevation,
                    )
                    .sqrt()
                        > 40.0
                    {
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
                //
                // Original's `BattleDecisions()` is fully synchronous, so
                // `mCurrentSubstate` is only read once every nested decision
                // has committed. Rust splits `ReconsiderEnemyApproach` around
                // its `GoNear` and finishes it from the owner FIFO — and that
                // continuation can still leave the any-swordfight set for
                // `SUBSTATE_ATTACKING_RUN_TO_AVENGER_ON_ROOF`. Queue the test
                // behind those continuations instead of reading a substate
                // that Original never observes.
                self.battle_decisions(sim, global, ctx, tick, grid);
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::TooProudOverviewFinallyFightRemark);
            }
            _ => {}
        }
        false
    }

    /// Owner-boundary tail of
    /// `SUBSTATE_ATTACKING_TOO_PROUD_TO_ATTACK_OVERVIEW`'s `EVENT_TIMER`
    /// arm (`RHartificialmalignity.cpp:4231-4245`): once `BattleDecisions()`
    /// has fully returned, a substate inside `_ANY_SWORDFIGHT_SUBSTATE_`
    /// earns the "finally, a fight" remark.
    pub(crate) fn too_proud_overview_finally_fight_remark(&mut self) {
        if self.base.current_substate.is_any_swordfight() {
            let remark = if self.is_vip {
                Remark::VipProudFinallyFight
            } else {
                Remark::ProudFinallyFight
            };
            self.base.say(remark);
        }
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
            let target_on_lift = grid.is_some_and(|g| {
                let position = tick.primary_target_live_position.unwrap_or_else(|| {
                    panic!(
                        "ladder-waiting soldier {} requires live position for primary target {}",
                        self.base.me, self.base.primary_target
                    )
                });
                let sector = position.sector.unwrap_or_else(|| {
                    panic!(
                        "ladder-waiting soldier {} requires a sector for primary target {}",
                        self.base.me, self.base.primary_target
                    )
                });
                g.sector_type(u32::from(sector)).is_lift()
            });
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
                let target_position = ctx
                    .expect_entity_view(
                        self.base.primary_target,
                        "detected avenger-on-roof primary target",
                    )
                    .position;
                self.base.face_position_3d_with_ctx(target_position, ctx);
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
}
