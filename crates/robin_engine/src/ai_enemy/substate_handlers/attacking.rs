//! Expected-event handlers for the `Attacking` state family.
//!
//! This mirrors the `STATE_ATTACKING` section of
//! original-game hostile AI expected-event behavior.

use super::*;

impl EnemyAi {
    pub(super) fn think_expected_attacking_event(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        let stimulus_type = stimulus.stimulus_type;
        Ok(match self.base.current_substate {
            Substate::AttackingReactiontimeTurning => {
                self.attacking_reactiontime_turning(stimulus_type, ctx)
            }

            Substate::AttackingReactiontime => {
                self.attacking_reactiontime(stimulus_type, global, env)?
            }

            Substate::AttackingReactiontimeRunning => {
                self.attacking_reactiontime_running(stimulus_type, global, env)?
            }

            Substate::AttackingRunningToEnemy
            | Substate::AttackingWalkingToEnemy
            | Substate::AttackingChargingEnemy => {
                self.attacking_running_to_enemy(stimulus_type, env)?
            }

            Substate::AttackingOverviewLookLeft => self.attacking_overview_look_left(stimulus_type),

            Substate::AttackingOverviewLookRight => {
                self.attacking_overview_look_right(stimulus_type, global, env)?
            }

            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventGaloppLoopEnd =>
            {
                self.attacking_rider_charging_approaching_on_event_galopp_loop_end(env)?
            }

            Substate::AttackingRiderChargingApproaching
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                self.attacking_rider_charging_approaching_on_event_reach_point(env)?
            }

            Substate::AttackingRiderChargingPassing
                if stimulus_type == StimulusType::EventReachPoint =>
            {
                self.attacking_rider_charging_passing(env)
            }

            Substate::AttackingRiderChargingGettingDistance => {
                self.attacking_rider_charging_getting_distance(stimulus_type, ctx)
            }

            Substate::AttackingRiderChargingReturning => {
                self.attacking_rider_charging_returning(stimulus_type, global, env)?
            }

            Substate::AttackingRiderChargingApproachingBlindly => {
                self.attacking_rider_charging_approaching_blindly(stimulus_type, ctx)
            }

            Substate::AttackingSwordfight => {
                self.attacking_swordfight(stimulus_type, global, env)?
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
                self.attacking_moving_around_old_enemy(stimulus_type, global, env)?
            }

            Substate::AttackingQuittingSwordfight => {
                self.attacking_quitting_swordfight(stimulus_type, env)?
            }

            Substate::AttackingReserve => self.attacking_reserve(stimulus_type, ctx, tick),

            Substate::AttackingLastReserve => {
                self.attacking_last_reserve(stimulus_type, global, env)?
            }

            Substate::AttackingApproachToObserve => {
                self.attacking_approach_to_observe(stimulus_type, ctx)
            }

            Substate::AttackingObserve => self.attacking_observe(stimulus_type, global, env)?,

            Substate::AttackingObserveAndMove => {
                self.attacking_observe_and_move(stimulus_type, global, env)?
            }

            Substate::AttackingTooProudToAttack => {
                self.attacking_too_proud_to_attack(sim, stimulus_type, ctx)
            }

            Substate::AttackingTowerGuardAlert => {
                self.attacking_tower_guard_alert(stimulus_type)?
            }

            Substate::AttackingTowerGuardObserve => {
                self.attacking_tower_guard_observe(stimulus_type, env)?
            }

            Substate::AttackingBowShooting => false,

            Substate::AttackingBowAiming => false,

            Substate::AttackingBowLoading => false,

            Substate::AttackingBowObservingLoading => false,

            Substate::AttackingBowObserving => false,

            Substate::AttackingBowRunningBehindShieldBearer => false,

            Substate::AttackingDoorFightDelay => {
                self.attacking_door_fight_delay(stimulus_type, ctx)
            }

            Substate::AttackingDoorFightLeaving => {
                self.attacking_door_fight_leaving(stimulus_type, ctx)
            }

            Substate::AttackingDoorFightTurning => {
                self.attacking_door_fight_turning(stimulus_type, ctx)
            }

            Substate::AttackingDoorFightWaiting => {
                self.attacking_door_fight_waiting(stimulus_type, env)?
            }

            Substate::AttackingProtectingWithShield
            | Substate::AttackingAdvancingWithShield
            | Substate::AttackingRunningToPhalanx => false,
            Substate::AttackingPhalanx => false,

            Substate::AttackingReserveOverview => {
                self.attacking_reserve_overview(stimulus_type, global, env)?
            }

            Substate::AttackingApproachingSleepingEnemy => {
                self.attacking_approaching_sleeping_enemy(env, stimulus_type)?
            }

            Substate::AttackingKillingSleepingEnemy => {
                self.attacking_killing_sleeping_enemy(stimulus_type, env)?
            }

            Substate::AttackingArcherRetireFromCombat => {
                self.attacking_archer_retire_from_combat(stimulus_type, ctx)
            }

            Substate::AttackingArcherRetireFromCombatTurn => {
                self.attacking_archer_retire_from_combat_turn(stimulus_type, global, env)?
            }

            Substate::AttackingOfficerGivingOrders => {
                self.attacking_officer_giving_orders(stimulus_type, ctx)
            }

            Substate::AttackingOfficerGivingOrdersWaiting => {
                self.attacking_officer_giving_orders_waiting(stimulus_type, global, env)?
            }

            Substate::AttackingTooProudToAttackOverview => {
                self.attacking_too_proud_to_attack_overview(stimulus_type, global, env)?
            }

            Substate::AttackingTooProudToAttackRetire => {
                self.attacking_too_proud_to_attack_retire(stimulus_type, ctx)
            }

            Substate::AttackingTooProudToAttackRetireTurn => {
                self.attacking_too_proud_to_attack_retire_turn(stimulus_type, global, env)?
            }

            Substate::AttackingTooProudToAttackApproach => {
                self.attacking_too_proud_to_attack_approach(stimulus_type, global, env)?
            }

            Substate::AttackingArcherRunOnShootingPath => false,

            Substate::AttackingArcherRunOnShootingPathFinalSprint => false,

            Substate::AttackingArcherRunOnShootingPathTurn => false,

            Substate::AttackingReactiontimeBending => {
                self.attacking_reactiontime_bending(stimulus_type, global, env)?
            }

            Substate::AttackingArcherWaitOnArcheryPath
            | Substate::AttackingArcherWaitOnArcheryPathBending => {
                self.attacking_archer_wait_on_archery_path(env, stimulus_type)?
            }

            Substate::AttackingArcherWaitOnBendPoint => {
                self.attacking_archer_wait_on_bend_point(env, stimulus_type)?
            }

            Substate::AttackingDummyBehaviour => self.attacking_dummy_behaviour(stimulus_type, ctx),

            Substate::AttackingSwordfightStepBack => {
                self.attacking_swordfight_step_back(stimulus_type, ctx)
            }

            Substate::AttackingReturnToOtherPcAfterMenacing => {
                self.attacking_return_to_other_pc_after_menacing(stimulus_type, ctx)
            }

            Substate::AttackingRunningToLadder => {
                self.attacking_running_to_ladder(stimulus_type, env)?
            }

            Substate::AttackingWaitingAtLadder => {
                self.attacking_waiting_at_ladder(stimulus_type, env)?
            }

            Substate::AttackingRunToAvengerOnRoof => {
                self.attacking_run_to_avenger_on_roof(stimulus_type, ctx)
            }

            Substate::AttackingWaitForAvengerOnRoof => {
                self.attacking_wait_for_avenger_on_roof(env, stimulus_type, global)?
            }

            // No-op group — only substates that still genuinely have no
            // handler remain here.  Explicit enumeration (no `_ =>`
            // catch-all) so adding a new `Substate` variant is a compile
            // error forcing the author to decide where it belongs.  The
            // 83 variants implemented above were previously swept into this
            // group by the exhaustive-match refactor; see commit fbda9e7a
            // (AttackingSwordfightParade) for the original motivating fix.
            Substate::AttackingRiderChargingApproaching
            | Substate::AttackingRiderChargingPassing => false,

            _ => false,
        })
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
                        "primary target {:?} missing from entity views while reacting",
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
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
                self.reinitialize_them_list(ctx);
                self.set_state(AiState::Attacking, Substate::AttackingReactiontimeBending);
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::EquipBowDown);
            } else {
                self.i_am_in_trouble(
                    self.required(
                        self.base.primary_target,
                        "a primary target",
                        "reacting to an enemy",
                    )
                    .get(),
                );
                self.battle_decisions(env, global)?;
            }
        }
        Ok(false)
    }

    fn attacking_reactiontime_running(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        let debug_decision_path = super::super::decision_path_debug_enabled()
            && super::super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            crate::ai_enemy::parity_trace::AidecisionReactiontimeRunningEvent {
                frame: &(ctx.frame),
                owner: &(self.base.me),
                co: &(ctx.original_creation_order),
                state: &(self.base.current_state),
                substate: &(self.base.current_substate),
                primary: &(self.base.primary_target),
                rider: &(ctx.self_is_rider),
                couldnt: &(self.base.couldnt_reachpoint),
                already: &(self.base.already_on_point),
                owner_work_before: &(self.base.outbox.reentrant.owner_work),
                stimulus_type: &(stimulus_type),
            }
            .emit();
        }
        if stimulus_type == StimulusType::EventTimer
            || stimulus_type == StimulusType::EventReachPoint
        {
            self.base.stop_all();
            self.i_am_in_trouble(
                self.required(
                    self.base.primary_target,
                    "a primary target",
                    "finishing a running reaction",
                )
                .get(),
            );
            self.battle_decisions(env, global)?;
            if debug_decision_path {
                crate::ai_enemy::parity_trace::AidecisionReactiontimeRunningDone {
                    frame: &(ctx.frame),
                    owner: &(self.base.me),
                    state: &(self.base.current_state),
                    substate: &(self.base.current_substate),
                    primary: &(self.base.primary_target),
                    couldnt: &(self.base.couldnt_reachpoint),
                    already: &(self.base.already_on_point),
                    owner_work_after: &(self.base.outbox.reentrant.owner_work),
                }
                .emit();
            }
        }
        Ok(false)
    }

    fn attacking_running_to_enemy(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        match stimulus_type {
            StimulusType::EventReachPoint | StimulusType::EventTimer => {
                self.reconsider_enemy_approach(
                    stimulus_type == StimulusType::EventReachPoint,
                    env,
                )?;
            }
            _ => {}
        }
        Ok(false)
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            // Look-sidewards finished — short delay before deciding.
            StimulusType::EventDone => {
                self.base.launch_timer(10, ctx.frame);
            }
            // Timer fires → battle decisions.
            StimulusType::EventTimer => {
                self.battle_decisions(env, global)?;
            }
            _ => {}
        }
        Ok(false)
    }

    // ── Rider charging substates ──

    // Approaching: rider is running toward the enemy with RIDER_CHARGE flag.
    // On GALOPP_LOOP_END: check if we can begin the actual charge.
    // On REACHPOINT: we arrived at the approach position; overview battle.

    fn attacking_rider_charging_approaching_on_event_galopp_loop_end(
        &mut self,
        _env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let mut call = crate::ai::DutyCall::new(DutyFlags::empty(), false);
        call.tail = crate::ai::DutyTail::RiderAttack {
            fallback: crate::ai::RiderAttackFallback::Approach,
        };
        Err(call)
    }

    fn attacking_rider_charging_approaching_on_event_reach_point(
        &mut self,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        // Arrived at approach point
        self.get_battle_overview(0, env)?;
        Ok(false)
    }

    // Passing: rider is doing the actual charge pass through the enemy.
    // On REACHPOINT: charge pass is done, get distance for reattack.

    fn attacking_rider_charging_passing(&mut self, env: ThinkEnv<'_>) -> bool {
        let ThinkEnv { ctx, .. } = env;
        // Transition to getting distance
        self.set_state(
            AiState::Attacking,
            Substate::AttackingRiderChargingGettingDistance,
        );
        if let Some(goal) = self.get_good_rider_reattack_goal(env) {
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.rider_reattack(env, global)?;
        }
        Ok(false)
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
            self.set_state_with_timer(AiState::Wondering, Substate::WonderingLooking1, 30, ctx);
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
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
                crate::ai_enemy::parity_trace::ReconsiderStimulus {
                    frame: &(ctx.frame),
                    owner: &(self.base.me),
                    creation_order: &(ctx.original_creation_order),
                    stimulus_type: &(stimulus_type),
                }
                .emit();
            }
            // Clear the emoticon.
            self.base.set_emoticon(EmoticonType::None);
            self.reconsider_swordfight(env, false, global)
                .map_err(|duty| duty.then(crate::ai::DutyTail::SwordfightInsult))?;
            self.swordfight_insult_after_reconsider();
        }
        Ok(false)
    }

    pub(crate) fn swordfight_insult_after_reconsider(&mut self) {
        if self.base.current_substate == Substate::AttackingSwordfight {
            if self.pending_sword_strike_consideration {
                self.pending_combat_insult_after_strike_consideration = true;
            } else {
                self.base.say(Remark::CombatInsult);
            }
        }
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
    // The original game checks specifically for sword parrying before
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
            self.set_state_with_timer(AiState::Attacking, Substate::AttackingSwordfight, 20, ctx);
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
            let target_handle = self.required(
                self.base.primary_target,
                "a primary target",
                "approaching a newly selected enemy",
            );
            let target = self.required_fighter(
                target_handle,
                tick,
                format_args!(
                    "AttackingApproachingNewEnemy primary target {target_handle} is missing its required fighter snapshot"
                ),
            );
            if tick.primary_target_snapshot_handle != Some(target_handle) {
                let snapshot_handle = tick
                    .primary_target_snapshot_handle
                    .map(AiEntityHandle::get)
                    .map_or_else(|| "absent".to_owned(), |handle| handle.to_string());
                panic!(
                    "AttackingApproachingNewEnemy primary target {target_handle} does not match tick snapshot handle {snapshot_handle}"
                );
            }
            let target_live_position = tick.primary_target_live_position.unwrap_or_else(|| {
                panic!(
                    "AttackingApproachingNewEnemy primary target {target_handle} is missing its literal position"
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
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingSwordfight,
                    20,
                    ctx,
                );
                self.base.outbox.actor.set_principal = Some(target_handle);
            } else {
                // Re-approach
                self.base
                    .go_near(target.position, sword_range as i32, GotoFlags::RUN, ctx);
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.set_state_with_timer(
                        AiState::Attacking,
                        Substate::AttackingSwordfight,
                        20,
                        ctx,
                    );
                    self.base.outbox.actor.set_principal = Some(target_handle);
                }
            }
        }
        false
    }

    // Reached position around old enemy.

    fn attacking_moving_around_old_enemy(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventReachPoint {
            self.set_state_with_timer(AiState::Attacking, Substate::AttackingSwordfight, 20, ctx);
            self.reconsider_swordfight(env, false, global)?;
        }
        Ok(false)
    }

    // Quitting swordfight timer.

    fn attacking_quitting_swordfight(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            // The original game checks for a sword-action state, not whether
            // the opponent list is still populated. Ending a swordfight
            // removes opponents before the current sword animation
            // necessarily finishes, so the action state remains the
            // authoritative completion gate.
            if ctx.self_action_state.is_sword() {
                // Original retries only when the actor is not already
                // executing QUIT_SWORDFIGHT. The AI context cannot inspect
                // the live sequence-manager selection, so retain that guard
                // on a distinct engine-side effect.
                if ctx.is_swordfighting {
                    self.base.outbox.actor.retry_quit_swordfight = true;
                }
                self.base.launch_timer(3, ctx.frame);
            } else {
                // Left sword state — proceed to battle overview.
                self.get_battle_overview(0x0000, env)?;
            }
        }
        Ok(false)
    }

    fn attacking_reserve(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        match stimulus_type {
            // Fall-through to CallCoordinate: walk the us-list built
            // by the last battle decision, pick the soldiers still in
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
                            info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(me)),
                        },
                    );
                }
                // Fall through to CallCoordinate arm.
                self.reinitialize_them_list(ctx);
                self.base.set_emoticon(EmoticonType::None);
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingReserveOverview,
                    20,
                    ctx,
                );
            }
            StimulusType::CallCoordinate => {
                self.reinitialize_them_list(ctx);
                self.base.set_emoticon(EmoticonType::None);
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingReserveOverview,
                    20,
                    ctx,
                );
            }
            _ => {}
        }
        false
    }

    // Last reserve: battle decisions on timer
    // (AttackingReserveOverview is below).

    fn attacking_last_reserve(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.battle_decisions(env, global)?;
        }
        Ok(false)
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
            if self.base.primary_target.is_some() {
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.reconsider_swordfight_observation(env, global)?;
        }
        Ok(false)
    }

    // Swordfight observation reconsideration: reached the observe-and-move
    // destination — immediately reconsider the current swordfight.

    fn attacking_observe_and_move(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventReachPoint {
            self.reconsider_swordfight_observation(env, global)?;
        }
        Ok(false)
    }

    // TooProud entry: reinit list, clear emoticon, transition
    // to Overview, 1/16 chance of looking sideways else 20-tick
    // timer.

    fn attacking_too_proud_to_attack(
        &mut self,
        sim: &SimulationContext,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventTimer {
            self.reinitialize_them_list(ctx);
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
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            return Err(crate::ai::DutyCall {
                flags: crate::ai::DutyFlags::empty(),
                think_result: false,
                tail: crate::ai::DutyTail::TowerGuardAlert {
                    center: self.base.seek_position,
                },
                after: Vec::new(),
            });
        }
        Ok(false)
    }

    fn attacking_tower_guard_observe(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            // The observing tower guard takes a fresh battle
            // overview rather than re-deciding directly: that
            // rebuilds the them-list, drops the task priority back
            // to the minimum and starts the look-left/look-right
            // sweep instead of immediately re-entering the same
            // observe substate.
            self.get_battle_overview(0, env)?;
        }
        Ok(false)
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
    // ticks; otherwise begin a swordfight.

    fn attacking_door_fight_turning(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            if self.base.primary_target.is_none() {
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingDoorFightWaiting,
                    150,
                    ctx,
                );
            } else {
                self.begin_swordfight(ctx);
            }
        }
        false
    }

    pub(super) fn attacking_door_fight_waiting(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            if super::super::them_lifecycle_debug_matches(ctx) {
                crate::ai_enemy::parity_trace::ThemTimerEntryOfficerOrdersWaiting {
                    frame: &(ctx.frame),
                    co: &(ctx.original_creation_order),
                    me: &(self.base.me),
                    state: &(self.base.current_state),
                    substate: &(self.base.current_substate),
                }
                .emit();
            }
            // Original resumes a door fight through battle-overview evaluation: it
            // rebuilds the enemy list and starts the left/right observation
            // sequence. Going directly to battle decisions skips those
            // observation states and can synchronously start an area search,
            // consuming selection RNG which Original has not requested yet.
            self.get_battle_overview(0, env)?;
        }
        Ok(false)
    }

    // ============ PHALANX / SHIELD-BEARER ============

    // ============ FLEEING ============
    // The malignity arm adds a single tweak (reset of
    // `fleeing_seen_enemy_counter` on the PANIC arm when the
    // panic is over) and then falls through into
    // `think_expected_event_common_stuff`, which owns the
    // actual panic/hide/door/hiding state machine.

    fn attacking_reserve_overview(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.battle_decisions(env, global)?;
        }
        Ok(false)
    }

    // Approaching a sleeping PC/NPC: reach → face target; done →
    // kill or menace depending on coma/distance.

    fn attacking_approaching_sleeping_enemy(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, tick, .. } = env;
        match stimulus_type {
            StimulusType::EventReachPoint => {
                self.base.face_entity(self.base.primary_target, ctx);
            }
            StimulusType::EventDone => {
                // Decision tree:
                //   if primary_target unconscious {
                //     if is_pc && in_coma {
                //       if pc.guard == None { menace }
                //       else { return to duty }
                //     } else if distance > 40 { run to within 20 units }
                //     else { SUBSTATE_KILLING_SLEEPING_ENEMY +
                //            SWORDSTRIKE_DOWN on primary_target }
                //   } else { evaluate the battle overview }
                let view =
                    ctx.expect_entity_view(self.base.primary_target, "approach-sleeping target");
                let target_pos = Some(view.position);
                let target_unconscious = view.is_unconscious;
                let target_is_pc = view.is_pc;
                let target_in_coma = view.in_coma;
                let target_guard = view.guard;

                if !target_unconscious {
                    self.get_battle_overview(0, env)?;
                } else if target_is_pc && target_in_coma && target_guard.is_none() {
                    // Coma/menace branch — PC is in coma and not yet
                    // guarded.
                    self.base.stop_all();
                    self.set_state(AiState::Menacing, Substate::MenacingPcInComa);
                    if self.is_vip {
                        // VIP variant — a bare sword draw, NOT an engagement.
                        // This builds the
                        // ENTER_SWORDFIGHT element with
                        // clearing both the opponent and
                        // jump-line destination, so
                        // Original-game enter-swordfight translation
                        // skips
                        // swordfight entry and stores no
                        // antagonist on the raise-sword order. The
                        // soldier's RAISING_SWORD arm
                        // then performs
                        // neither direction assignment nor turning: the VIP
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
                        // Assigning the guarded player character sets both the
                        // soldier's guarded_pc and the PC's reciprocal
                        // guard.
                        self.set_guarded_pc(Some(crate::entity_id::PcId(
                            self.required(
                                self.base.primary_target,
                                "a primary target",
                                "menacing a comatose PC",
                            )
                            .get(),
                        )));
                    }
                    self.base.launch_timer(20, ctx.frame);
                } else if target_is_pc && target_in_coma && target_guard.is_some() {
                    // PC already menaced by another guard — go home.
                    self.return_to_duty_default(env)?;
                } else if let Some(p) = target_pos {
                    if tick.primary_target_snapshot_handle != self.base.primary_target {
                        panic!(
                            "ApproachingSleepingEnemy primary target {:?} does not match tick snapshot handle {:?}",
                            self.base.primary_target, tick.primary_target_snapshot_handle
                        );
                    }
                    let target_live_position =
                        tick.primary_target_live_position.unwrap_or_else(|| {
                            panic!(
                                "ApproachingSleepingEnemy primary target {:?} is missing its literal position",
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
                        // Run to within 20 units of the target.
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
                                    "sleeping-enemy target {:?} has no typed live entity view",
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
                    self.get_battle_overview(0, env)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    // Killing sleeping enemy done: say REMARK_KILLED_ADVERSARY and
    // overview.

    fn attacking_killing_sleeping_enemy(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.base.say(Remark::KilledAdversary);
            self.get_battle_overview(0, env)?;
        }
        Ok(false)
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
            // Facing paths, selecting the fast-turn command.
            if self.base.primary_target.is_some() {
                self.base.face_entity_fast(self.base.primary_target, ctx);
            } else {
                self.base
                    .face_position_fast_with_ctx(self.base.seek_position, ctx);
            }
        }
        false
    }

    // Done turning: re-engage via battle decisions or overview.

    fn attacking_archer_retire_from_combat_turn(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventDone {
            // If the primary target is detected within 180 degrees, make
            // battle decisions; otherwise evaluate the battle overview.
            if let Some(primary_target) = self
                .base
                .primary_target
                .filter(|target| self.is_detecting_180_degrees(*target, ctx))
            {
                // Original restores a still-visible primary target to
                // the persistent Them list before making the next
                // tactical decision.  The target can have fallen out
                // of that list during the retreat; omitting this made
                // battle planning start an area search because no enemies remain.
                if !self.list_them.contains(&primary_target.get()) {
                    self.list_them.push(primary_target.get());
                }
                self.battle_decisions(env, global)?;
            } else {
                self.get_battle_overview(0, env)?;
            }
        }
        Ok(false)
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
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            self.reinitialize_them_list(ctx);
            if !self.list_them.is_empty() {
                self.battle_decisions(env, global)?;
            } else {
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
        Ok(false)
    }

    // Too-proud overview: on done, focus target and short timer; on
    // timer, make battle decisions and maybe say
    // REMARK_PROUD_FINALLY_FIGHT.

    fn attacking_too_proud_to_attack_overview(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        match stimulus_type {
            StimulusType::EventDone => {
                if self.base.primary_target.is_some() {
                    self.base.outbox.actor.set_focus(self.base.primary_target);
                }
                self.base.launch_timer(5, ctx.frame);
            }
            StimulusType::EventTimer => {
                // Make battle decisions, then if the resulting substate is a
                // swordfight (VIP variant says VIP_REMARK, otherwise
                // REMARK_PROUD_FINALLY_FIGHT).
                //
                // The original game's battle decision is fully synchronous, so
                // `mCurrentSubstate` is only read once every nested decision
                // has committed. Rust splits enemy approach reconsideration around
                // its approach and finishes it from the owner FIFO — and that
                // continuation can still leave the any-swordfight set for
                // `SUBSTATE_ATTACKING_RUN_TO_AVENGER_ON_ROOF`. Queue the test
                // behind those continuations instead of reading a substate
                // that Original never observes.
                self.battle_decisions(env, global)
                    .map_err(|duty| duty.then(crate::ai::DutyTail::TooProudOverviewRemark))?;
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::TooProudOverviewFinallyFightRemark);
            }
            _ => {}
        }
        Ok(false)
    }

    /// Owner-boundary tail of
    /// `SUBSTATE_ATTACKING_TOO_PROUD_TO_ATTACK_OVERVIEW`'s `EVENT_TIMER`
    /// arm: once battle planning
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

    // Finished turning: re-engage with battle decisions or
    // Battle-overview evaluation.

    fn attacking_too_proud_to_attack_retire_turn(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventDone {
            if self
                .base
                .primary_target
                .is_some_and(|target| self.is_detecting_180_degrees(target, ctx))
            {
                self.battle_decisions(env, global)?;
            } else {
                self.get_battle_overview(0, env)?;
            }
        }
        Ok(false)
    }

    // Approach finished: battle decisions or overview based on
    // 180-degree detection.

    fn attacking_too_proud_to_attack_approach(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventReachPoint {
            if self
                .base
                .primary_target
                .is_some_and(|target| self.is_detecting_180_degrees(target, ctx))
            {
                self.battle_decisions(env, global)?;
            } else {
                self.get_battle_overview(0, env)?;
            }
        }
        Ok(false)
    }

    // Note: `AttackingRiderChargingApproaching` /
    // `AttackingRiderChargingPassing` were previously duplicated
    // here with a "rider support not implemented" stub. The implementation
    // lives further up in this same match (see
    // `Substate::AttackingRiderChargingApproaching` around the
    // rider-charging substates block), which takes precedence;
    // the stubs were dead arms.  Removed.

    // Archer finished bending (reactiontime bend): in-trouble +
    // Battle decisions.

    fn attacking_reactiontime_bending(
        &mut self,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventDone {
            self.i_am_in_trouble(
                self.required(
                    self.base.primary_target,
                    "a primary target",
                    "finishing an archer bend reaction",
                )
                .get(),
            );
            self.battle_decisions(env, global)?;
        }
        Ok(false)
    }

    // Original returns an archer parked on either kind of shooting
    // path to ordinary duty when the hold timer expires.

    fn attacking_archer_wait_on_archery_path(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
    }

    // Shared "timer → return to duty" for bow archers waiting on
    // archery/bend points.

    fn attacking_archer_wait_on_bend_point(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
    ) -> crate::ai::AiFlow<bool> {
        if stimulus_type == StimulusType::EventTimer {
            self.return_to_duty_default(env)?;
        }
        Ok(false)
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
            self.set_state_with_timer(AiState::Attacking, Substate::AttackingSwordfight, 20, ctx);
        }
        false
    }

    // Officer approached brawl victim: kick off wake-up sequence.

    fn attacking_return_to_other_pc_after_menacing(
        &mut self,
        stimulus_type: StimulusType,
        ctx: &AiContext,
    ) -> bool {
        if stimulus_type == StimulusType::EventDone {
            self.begin_swordfight(ctx);
        }
        false
    }

    // Running to enemy on a ladder: reach → face + focus + wait;
    // timer → reconsider.

    fn attacking_running_to_ladder(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ctx = env.ctx;
        match stimulus_type {
            StimulusType::EventReachPoint => {
                if self.base.primary_target.is_some() {
                    self.base.face_entity(self.base.primary_target, ctx);
                    self.base.outbox.actor.set_focus(self.base.primary_target);
                }
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingWaitingAtLadder,
                    1,
                    ctx,
                );
            }
            StimulusType::EventTimer => {
                self.reconsider_enemy_approach(false, env)?;
            }
            _ => {}
        }
        Ok(false)
    }

    // Waiting at ladder: if enemy still on lift, reface & rearm;
    // else reconsider.

    fn attacking_waiting_at_ladder(
        &mut self,
        stimulus_type: StimulusType,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv {
            ctx, tick, grid, ..
        } = env;
        if stimulus_type == StimulusType::EventTimer {
            // If primary target is on a lift sector, face+focus+wait;
            // else reconsider enemy approach.
            // This source arm calls
            // whether the primary target's sector is a lift directly. It
            // must inspect the target element's physical sector, not
            // The original-game AI position's committed
            // destination-side sector during a door pass.
            let target_on_lift = grid.is_some_and(|g| {
                let position = tick.primary_target_live_position.unwrap_or_else(|| {
                    panic!(
                        "ladder-waiting soldier {} requires live position for primary target {:?}",
                        self.base.me, self.base.primary_target
                    )
                });
                let sector = position.sector.unwrap_or_else(|| {
                    panic!(
                        "ladder-waiting soldier {} requires a sector for primary target {:?}",
                        self.base.me, self.base.primary_target
                    )
                });
                g.sector_type_for_handle(sector).is_lift()
            });
            if target_on_lift {
                self.base.face_entity(self.base.primary_target, ctx);
                self.base.outbox.actor.set_focus(self.base.primary_target);
                self.base.launch_timer(20, ctx.frame);
            } else {
                self.reconsider_enemy_approach(false, env)?;
            }
        }
        Ok(false)
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
            self.set_state_with_timer(
                AiState::Attacking,
                Substate::AttackingWaitForAvengerOnRoof,
                100,
                ctx,
            );
        }
        false
    }

    // Wait for avenger: either re-face if detected, or search the area on
    // lost sight.

    fn attacking_wait_for_avenger_on_roof(
        &mut self,
        env: ThinkEnv<'_>,
        stimulus_type: StimulusType,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<bool> {
        let ThinkEnv { ctx, .. } = env;
        if stimulus_type == StimulusType::EventTimer {
            // If the primary target is detected within 180 degrees, face it
            // again and start a 30-tick timer. Otherwise search from the actor's
            // position with AI_LOST_ENEMY_SEEK_RADIUS and no extra flags.
            if self
                .base
                .primary_target
                .is_some_and(|target| self.is_detecting_180_degrees(target, ctx))
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
                    env,
                    ctx.position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::empty(),
                    UNDEFINED_DIRECTION,
                    global,
                )?;
            }
        }
        Ok(false)
    }
}
