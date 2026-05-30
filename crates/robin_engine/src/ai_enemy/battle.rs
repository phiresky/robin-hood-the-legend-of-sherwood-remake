//! Battle decisions and combat execution.
//!
//! Contains the combat decision tree (`battle_decisions`,
//! `make_battle_predecisions`, `execute_battle_decision`,
//! `get_battle_overview`), enemy approach (`attack_enemy`,
//! `reconsider_enemy_approach`), rider charges (`maybe_make_rider_attack`
//! and helpers), the sleeping-enemy approach helpers, and the
//! swordfight begin/end transitions.

use crate::ai::*;
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

use super::util::{
    dot2, iso_norm, max_norm, pos_diff, sector_to_vector_iso, square_norm, vec_to_sector,
};
use super::{
    EnemyAi, FighterSnapshot, PrimaryTargetFlags, ProfileRank, SeekFlags, UNDEFINED_DIRECTION,
    archer, combat,
};

impl EnemyAi {
    // -----------------------------------------------------------------------
    // approach_sleeping_enemies
    // -----------------------------------------------------------------------

    /// Move the supplied unconscious enemies into `list_them`, pick
    /// the nearest one as primary target, and walk up to finish them
    /// off.  If no allowed target is found, fall back to `ReturnToDuty`.
    ///
    /// `targets` is typically `tick.unconscious_enemies` (the
    /// "already-seen-then-knocked-out" path from `BattleDecisions`)
    /// or `tick.nearby_sleeping_enemies` (the
    /// `KillNearbySleepingEnemies` fallback). The two paths share the
    /// exact same tail end — only the source of the list differs.
    fn approach_sleeping_enemies(
        &mut self,
        targets: &[crate::ai::SleepingEnemyInfo],
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Fold the sleeping-enemy list into list_them so the later
        // combat selection code sees them.
        for se in targets {
            if !self.list_them.contains(&se.handle) {
                self.list_them.push(se.handle);
            }
        }

        // Pick the nearest allowed target. The reference calls
        // `GetNewPrimaryTarget()` here — that function walks `list_them`
        // and picks the lowest distance (plus a multiplicity penalty).
        // In our port the cached `enemy_sq_distances` doesn't contain
        // unconscious enemies (they were filtered out at snapshot time),
        // so we do the same distance pick directly against the
        // sleeping-enemy list. VIP / mission rules still apply.
        let my_pos = ctx.position;
        let mut best: Option<(HumanHandle, crate::ai::Position)> = None;
        let mut best_sq: f32 = f32::MAX;
        for se in targets {
            // IsAllowedToAttack: VIPs can only fight Robin, and nobody can
            // start a fight with a VIP NPC. Sleeping enemies are always
            // PCs in our current scan, but we still honour the VIP rules
            // so heroes in a VIP-only mission can't be finished off by
            // regular soldiers.
            if self.is_vip && (!se.is_pc || !se.is_robin) {
                continue;
            }
            if !se.is_pc && se.is_vip {
                continue;
            }
            let dx = se.position.x - my_pos.x;
            let dy = se.position.y - my_pos.y;
            let sq = dx * dx + dy * dy;
            if sq < best_sq {
                best_sq = sq;
                best = Some((se.handle, se.position));
            }
        }

        if let Some((target_handle, target_pos)) = best {
            // SetState(Attacking, ApproachingSleepingEnemy) +
            // GoNear(target_pos, 20, RUN).
            self.base.primary_target = target_handle;
            self.go_near(
                AiState::Attacking,
                Substate::AttackingApproachingSleepingEnemy,
                target_pos,
                20,
                GotoFlags::RUN,
                ctx,
            );
        } else {
            // No allowed target — stand down.
            self.return_to_duty(DutyFlags::empty(), ctx, tick);
        }
    }

    // -----------------------------------------------------------------------
    // KillNearbySleepingEnemies
    // -----------------------------------------------------------------------

    /// Final fallback from `BattleDecisions` when the NPC has nothing
    /// else to do: scan the nearby area for unconscious enemies and
    /// walk over to finish one off.
    ///
    /// The nearby-enemy scan is performed by the engine during
    /// tick-data population and surfaced via
    /// `tick.nearby_sleeping_enemies`.  This method just performs
    /// the target selection + state transition that the reference
    /// runs after the inline GetNumberOfFighters loop.
    fn kill_nearby_sleeping_enemies(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        // Combat trainers and merry-man-forest fighters call
        // `ReturnToDuty()` first — note the quirk that the function then
        // *continues* and may still overwrite state with
        // `SUBSTATE_ATTACKING_APPROACHING_SLEEPING_ENEMY` below. We
        // mirror the behaviour exactly.
        if self.combat_trainer || self.is_merry_man_forest(ctx) {
            self.return_to_duty(DutyFlags::empty(), ctx, tick);
        }

        // Clear the them-list before refilling it with sleeping bodies.
        self.list_them.clear();

        // The engine-side scan already handled the
        // `IsDetecting360Degrees` + non-carried + layer check.
        // Everything we see here is a valid finish-off candidate.
        self.approach_sleeping_enemies(&tick.nearby_sleeping_enemies, ctx, tick);
    }

    // -----------------------------------------------------------------------
    // FillListWithAllNearFighters
    //
    // Fills `list` with fighters from `tick.nearby_fighters` that belong
    // to the requested camp side relative to `me` and are within the
    // MAX_SWORDFIGHT_CONSIDERATION_RADIUS (=500 MaxNorm) — the radius
    // filter was already applied when the snapshot was built.
    //
    // When `is_my_camp` is true we seed the list with `me` and require
    // `is_swordfighting` on other entries. When false (enemy camp) any
    // able-to-fight opponent counts. Returns `true` iff the list is
    // non-empty.
    // -----------------------------------------------------------------------
    fn fill_list_with_all_near_fighters(
        list: &mut Vec<HumanHandle>,
        me: HumanHandle,
        is_my_camp: bool,
        tick: &AiPerTickData,
    ) -> bool {
        list.clear();

        let must_be_swordfighting = is_my_camp;
        if is_my_camp {
            list.push(me);
        }

        for f in &tick.nearby_fighters {
            // `is_friendly` in the snapshot reflects same-camp
            // membership relative to the scanning NPC.
            if f.is_friendly != is_my_camp {
                continue;
            }
            if f.handle == me {
                continue;
            }
            if !f.is_able_to_fight {
                continue;
            }
            if must_be_swordfighting && !f.is_swordfighting {
                continue;
            }
            list.push(f.handle);
        }

        !list.is_empty()
    }

    // -----------------------------------------------------------------------
    // GetBattleOverview
    // -----------------------------------------------------------------------

    pub fn get_battle_overview(&mut self, flags: u16, ctx: &AiContext, tick: &AiPerTickData) {
        const FAST_OVERVIEW: u16 = 0x0001;

        if (flags & FAST_OVERVIEW) != 0 {
            // FillListWithAllNearFighters(list_them, enemyCamp) uses
            // `must_be_swordfighting = false`, i.e. the FAST gate fires
            // whenever *any* able-to-fight enemy is within the 500-
            // MaxNorm radius, regardless of swordfighting state.
            let me = self.base.me;
            if Self::fill_list_with_all_near_fighters(&mut self.list_them, me, false, tick) {
                // Rebuild our-list with swordfighting-only friends on the
                // same camp (self is seeded first).
                Self::fill_list_with_all_near_fighters(&mut self.base.list_us, me, true, tick);

                let target = self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick);
                if target != 0 {
                    self.base.primary_target = target;
                    self.attack_enemy(target, None, ctx, tick, None);
                    return;
                }
            }
        }

        self.reinitialize_them_list(ctx, tick);
        self.current_task_priority = self.minimal_task_priority;

        self.set_state(AiState::Attacking, Substate::AttackingOverviewLookLeft);
        self.base.stop_all();
        // LOOK_LEFT kicks off the overview glance sequence before the
        // right-glance transition.
        self.base.pending_look_sidewards = Some(LookDirection::Left);
    }

    // -----------------------------------------------------------------------
    // MakeBattlePredecisions — offensive or defensive?
    // -----------------------------------------------------------------------

    pub fn make_battle_predecisions(&mut self, ctx: &AiContext, tick: &AiPerTickData) -> Decision {
        // Archers with no ammo or already swordfighting → defensive.
        if self.is_archer() && (ctx.remaining_arrows == 0 || ctx.is_swordfighting) {
            return Decision::PredecisionDefensive;
        }

        // Already fleeing → defensive
        if self.base.current_state == AiState::Fleeing {
            return Decision::PredecisionDefensive;
        }

        // --------- US ---------
        // Points include pride for soldiers (100 + pride), flat 100 for
        // PCs. Pre-computed by the engine tick into
        // `tick.us_battle_points`.
        let us_points: u32 = tick.us_battle_points.max(100);

        // --------- THEM ---------
        // Enemies contribute 100 each. Zero-enemy case never reaches
        // here (battle_decisions returns early), so no .max(1) needed.
        let them_points: u32 = self.list_them.len() as u32 * 100;

        // --------- EVALUATION ---------
        // relation_times_100 = (us_points * 100) / (them_points + 1)
        let relation_times_100 = (us_points * 100) / (them_points + 1);
        let mut odds: i16 = if relation_times_100 >= 100 {
            let raw = 50
                + (50 * (relation_times_100 - 100) as i32)
                    / parameters_ai::AI_BEST_BATTLE_RELATION_MINUS_100;
            raw.min(100) as i16
        } else {
            let raw = (50 * (relation_times_100 as i32 - parameters_ai::AI_WORST_BATTLE_RELATION))
                / parameters_ai::AI_100_MINUS_WORST_BATTLE_RELATION;
            raw.max(0) as i16
        };

        // Wounded soldiers are more pessimistic.
        let max_lp = self.initial_life_points.max(1);
        let cur_lp = self.old_life_points;
        if cur_lp < max_lp {
            odds = (odds as i32 * cur_lp as i32 / max_lp as i32) as i16;
        }

        // Officer nearby bonus (multiplicative): with OFFICER_ODDS_BONUS
        // = 30, soldiers with an officer nearby almost always choose
        // offensive behaviour.
        if self.get_rank() == ProfileRank::Soldier && tick.has_officer_nearby {
            odds = (odds as i32 * combat::OFFICER_ODDS_BONUS).min(i16::MAX as i32) as i16;
        }

        self.old_odds = odds;

        // Decision based on odds and courage.
        let courage = self.get_courage();
        if odds < (50 - courage as i16 / 2) && crate::sim_rng::u16(0..100) > courage {
            Decision::PredecisionDefensive
        } else {
            Decision::PredecisionOffensive
        }
    }

    // -----------------------------------------------------------------------
    // BattleDecisions — the heart of tactical AI
    // -----------------------------------------------------------------------

    pub fn battle_decisions(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        tracing::trace!(
            me = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            "battle_decisions: entry"
        );
        // Focus(NULL) at BattleDecisions entry. The decision tree will
        // re-focus on a freshly chosen primary target later (via
        // `pending_focus`) if it picks Fight / Shoot / etc.
        self.base.pending_unfocus = true;

        // Rebuild list_us from the global same-camp fighter registry on
        // entry. Do the same from the engine snapshot so deferred
        // EVENT_VIEW and timer paths cannot reuse a stale us-list.
        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        for friend in &tick.nearby_fighters {
            if !friend.is_friendly || friend.handle == self.base.me || !friend.is_able_to_fight {
                continue;
            }
            if friend.is_pc
                || matches!(
                    friend.ai_state,
                    AiState::Default | AiState::Wondering | AiState::Seeking | AiState::Attacking
                )
            {
                self.base.list_us.push(friend.handle);
            }
        }

        // Clean up them list — remove dead/unable enemies
        // Refresh `list_them` from the tick's detection snapshot so a
        // dispatch arriving with populated `enemy_sq_distances` (e.g.
        // the timer path's primary_target seed in engine/ai.rs:5898)
        // replaces a stale empty list.  Without this `battle_decisions`
        // reads the residual list, sees zero visible enemies, and
        // bails to `return_to_duty` — the "second guy detected me but
        // stood there" reactiontime ping-pong.  Carries primary_target
        // across when the tick snapshot doesn't include it (handled
        // inside `reinitialize_them_list`).
        self.reinitialize_them_list(ctx, tick);

        self.list_them.retain(|&h| h != 0); // basic cleanup

        // `num_enemies_i_can_see` is captured BEFORE friend-seen enemies
        // are injected. This count gates the offensive-decision block;
        // the merged total (personal + friend-seen) gates the
        // friend-seen-only seek arm.
        //
        // ReinitializeThemList includes unconscious enemies; the cleanup
        // pass below removes them and decrements
        // `num_enemies_i_can_see` for each one that fell within the
        // pre-cleanup window. Both halves of that pass run AFTER the
        // friend-seen injection.
        let mut num_enemies_i_can_see = self.list_them.len();

        // Walk same-camp soldiers in STATE_ATTACKING and inject their
        // primary target into list_them so we hunt where they are
        // fighting. Skip self, missing primary_target, and anything
        // already in our list. The injection happens AFTER
        // num_enemies_i_can_see is captured.
        {
            let me = self.base.me;
            let mut friend_seen: Vec<HumanHandle> = Vec::new();
            for cs in &tick.camp_soldiers {
                if cs.handle == me {
                    continue;
                }
                if cs.ai_state != AiState::Attacking {
                    continue;
                }
                let target = self
                    .find_fighter(cs.handle as HumanHandle, tick)
                    .map(|f| f.primary_target)
                    .unwrap_or(0);
                if target == 0 || target == me {
                    continue;
                }
                if self.list_them.contains(&target) || friend_seen.contains(&target) {
                    continue;
                }
                // Don't inject same-camp by accident — primary_target
                // can briefly be a same-camp during the cross-camp
                // setup race; skip if the target maps to a friendly
                // fighter snapshot.
                let target_is_friendly = self
                    .find_fighter(target, tick)
                    .map(|f| f.is_friendly)
                    .unwrap_or(false);
                if target_is_friendly {
                    continue;
                }
                friend_seen.push(target);
            }
            self.list_them.extend(friend_seen);
        }

        // Clean up the Them list. Walk each entry: if it's not
        // able-to-fight, drop it. Each removal that falls within
        // `num_enemies_i_can_see` decrements the personally-visible
        // counter. Friends accidentally on the list are also dropped.
        {
            let mut idx = 0;
            while idx < self.list_them.len() {
                let h = self.list_them[idx];
                let drop_entry = match ctx.entity_view(h) {
                    Some(view) => view.camp == ctx.camp || !view.is_able_to_fight,
                    None => {
                        tracing::warn!(
                            me = self.base.me,
                            target = h,
                            "battle_decisions: dropping them-list entry missing from entity view"
                        );
                        true
                    }
                };
                if drop_entry {
                    if idx < num_enemies_i_can_see {
                        num_enemies_i_can_see -= 1;
                    }
                    self.list_them.remove(idx);
                    continue;
                }
                idx += 1;
            }
        }

        // Get primary target
        self.base.primary_target =
            self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick);

        if num_enemies_i_can_see == 0 {
            // No visible enemies. Ordering:
            //   combat_trainer → my_shooting_point → archer-leaning-out
            //   → friends-see-enemies (seek) → missed-PC → unconscious
            //   → kill_nearby_sleeping. archer-leaning-out MUST come
            //   before the seek-friends-enemies arm — an archer parked
            //   on a bend point with friend-seen enemies should hold
            //   the firing position, not run away to seek.
            if self.combat_trainer {
                self.return_to_duty(DutyFlags::empty(), ctx, tick);
            } else if self.my_shooting_point.is_some() {
                // Archer has a shooting point — equip bow based on
                // elevation relative to last-seen enemy.
                let my_elevation: u16 = ctx.elevation as u16;
                if my_elevation >= self.enemy_had_this_elevation + 50 {
                    // Target is below — aim down
                    self.base
                        .pending_launch_commands
                        .push(crate::element::Command::EquipBowDown);
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingArcherWaitOnArcheryPathBending,
                    );
                } else {
                    // Target is at same level or above
                    self.base
                        .pending_launch_commands
                        .push(crate::element::Command::EquipBow);
                    self.set_state(
                        AiState::Attacking,
                        Substate::AttackingArcherWaitOnArcheryPath,
                    );
                }
                self.base.launch_timer(1000, ctx.frame);
            } else if self.enemy_seen_below
                && self.is_archer()
                && ctx.posture == crate::element::Posture::LeaningOut
            {
                // Archer leaning out saw enemy below; hold the bend point.
                // Must precede the friend-seen seek arm so an archer
                // mid-shot doesn't abandon his position to chase someone
                // else's sighting.
                self.set_state(AiState::Attacking, Substate::AttackingArcherWaitOnBendPoint);
                self.base.launch_timer(500, ctx.frame);
            } else if !self.list_them.is_empty() {
                // Friends see enemies that I don't — seek toward the
                // first friend's enemy position.
                if let Some(first_enemy) = self.list_them.first().copied()
                    && let Some(pos) = self
                        .find_fighter(first_enemy, tick)
                        .map(|f| f.position)
                        .or_else(|| ctx.entity_view(first_enemy).map(|v| v.position))
                {
                    self.base.seek_position = pos;
                }
                self.seek_area(
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            } else if self.pc_missed
                && self.missed_pc != 0
                && tick.missed_pc_is_pc
                && self.answer_question(Question::ShallIFollowLostEnemy, ctx)
            {
                // Lost enemy — re-forecast and seek with direction hint.
                self.base.say(Remark::HuntsEnemy);
                // Re-predict missed PC's destination before seeking.
                if let Some(forecast) = tick.missed_pc_forecast {
                    self.base.seek_position = forecast.position;
                    self.pc_gone_away_in_this_direction = forecast.direction;
                }
                self.seek_area(
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                    self.pc_gone_away_in_this_direction,
                    global,
                    ctx,
                    tick,
                );
            } else if !tick.unconscious_enemies.is_empty() && !self.is_merry_man_forest(ctx) {
                // Enemies I saw this tick are all unconscious and not
                // being carried — dump them into list_them, pick a
                // target, and walk up to finish them off.
                debug_assert!(self.list_them.is_empty());
                self.approach_sleeping_enemies(&tick.unconscious_enemies, ctx, tick);
            } else {
                // Final "there is literally nothing going on" fallback —
                // look for sleeping enemies anywhere within the 360°
                // detection radius and walk over to one.
                self.kill_nearby_sleeping_enemies(ctx, tick);
            }
            return;
        }

        // Determine decision
        let decision;
        // Shield bearer handle for CoverBehindShieldBearer decision.
        // Set during the decision-making phase, consumed by execution.
        let mut cover_shield_bearer: HumanHandle = 0;

        // Has the decision been forced?
        if self.forced_next_battle_decision != Decision::None {
            // Only a whitelist of decisions can be forced; the rest
            // assert (release-mode no-op, but worth keeping the guard so
            // scripts/debug paths don't silently take an unsupported
            // decision). The forbidden set is `AlertSoldiers`,
            // `RunAndAlertSoldiers`, `LookForHelpIfNobodyElseDoes`,
            // `CoverBehindShieldBearer`, `RunToArcheryPoint` — fall
            // back to the predecision flow rather than trusting the
            // forced value.
            let forced = self.forced_next_battle_decision;
            self.forced_next_battle_decision = Decision::None;
            let forced_allowed = matches!(
                forced,
                Decision::Cassos
                    | Decision::Fight
                    | Decision::Observe
                    | Decision::Reserve
                    | Decision::Menace
                    | Decision::Shoot
                    | Decision::ArcherStepBack
                    | Decision::LookForHelp
                    | Decision::TooProudToAttack
                    | Decision::TowerGuardAlert
                    | Decision::TowerGuardObserve
                    | Decision::ArcherObserve
            );
            if forced_allowed {
                decision = forced;
            } else {
                tracing::warn!(
                    me = self.base.me,
                    ?forced,
                    "battle_decisions: forced decision not in whitelist; falling back to predecision"
                );
                // Fall through to predecision flow as a release-mode
                // best-effort recovery.  We re-enter the else branch
                // by recording None below.
                self.forced_next_battle_decision = Decision::None;
                // Simulate "no forced decision" by jumping into the
                // else block via a goto-style early flag.
                let predecision = self.make_battle_predecisions(ctx, tick);
                decision = if self.combat_trainer || predecision == Decision::PredecisionDefensive {
                    Decision::Cassos
                } else {
                    Decision::Fight
                };
            }
        } else {
            // (1) Predecision: Offensive or defensive?
            let predecision = self.make_battle_predecisions(ctx, tick);

            // Use engine-populated cached values for battle context.
            let friends_with_lower_company = tick.friends_lower_company;
            let soldiers_with_lower_pride = tick.soldiers_lower_pride;
            let min_square_enemy_distance = tick.min_sq_enemy_distance;

            if self.combat_trainer {
                decision = Decision::Observe;
            } else if predecision == Decision::PredecisionOffensive {
                ////////// offensive decisions //////////////

                if self.is_archer() && self.base.blood_alcohol == 0 {
                    // Archer offensive.
                    if self.tower_guard {
                        if !self.base.friends_are_alerted {
                            decision = Decision::TowerGuardAlert;
                        } else {
                            decision = Decision::Shoot;
                        }
                    } else if self.base.primary_target != 0
                        && self.archer_is_too_near_to_enemy(
                            &ctx.position,
                            self.base.primary_target,
                            ctx,
                            tick,
                        )
                    {
                        // Step back and decide again.
                        decision = Decision::ArcherStepBack;
                    } else if self.shield_bearer_before_me != 0 && self.base.blood_alcohol == 0 {
                        // Already paired with a shield bearer — check if
                        // we're still in cover or need to reposition.
                        if let Some(cover_pos) = self.compute_position_behind_shield_bearer(
                            self.shield_bearer_before_me,
                            ctx,
                            tick,
                            grid,
                        ) {
                            let diff = pos_diff(&ctx.position, &cover_pos);
                            if max_norm(diff) < archer::COVER_POINT_TOLERANCE as f32 {
                                // Still in cover — shoot
                                decision = Decision::Shoot;
                            } else {
                                // Need to reposition behind shield bearer
                                cover_shield_bearer = self.shield_bearer_before_me;
                                decision = Decision::CoverBehindShieldBearer;
                            }
                        } else {
                            // Shield bearer lost or unreachable
                            self.update_shield_bearer_before_me(0);
                            decision = Decision::Shoot;
                        }
                    } else if self.my_shooting_point.is_some() {
                        // Already have a shooting point.
                        decision = Decision::Shoot;
                    } else if self.choose_good_shooting_point(global, ctx, tick) {
                        // Found a good archery point — run to it.
                        decision = Decision::RunToArcheryPoint;
                    } else {
                        // Search for a shield bearer to hide behind.
                        if let Some(sb) = self.get_nearest_free_shield_bearer(ctx, tick) {
                            cover_shield_bearer = sb;
                            decision = Decision::CoverBehindShieldBearer;
                        } else {
                            // No shield to hide behind
                            decision = Decision::Shoot;
                        }
                    }
                } else if self.tower_guard {
                    // Tower guard offensive.
                    if !self.base.friends_are_alerted {
                        decision = Decision::TowerGuardAlert;
                    } else if min_square_enemy_distance < combat::MIN_SQUARE_RESERVE_DISTANCE {
                        decision = Decision::Fight;
                    } else {
                        decision = Decision::TowerGuardObserve;
                    }
                } else if self.get_rank() == ProfileRank::Officer
                    && tick.simple_soldiers_near
                    && !self.base.friends_are_alerted
                    && self.base.blood_alcohol == 0
                {
                    // Officer alerts soldiers (only if simple soldiers are nearby).
                    decision = Decision::AlertSoldiers;
                } else if friends_with_lower_company >= self.list_them.len() as u16
                    && min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE
                {
                    // Enough friends closer → hold back.
                    decision = Decision::Reserve;
                } else if self.company_number == 100
                    && min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE
                {
                    // Company 100 → last reserve.
                    decision = Decision::LastReserve;
                } else if soldiers_with_lower_pride && self.is_too_proud_to_attack(ctx, tick) {
                    // Too proud to fight alongside commoners.
                    decision = Decision::TooProudToAttack;
                } else if ctx.camp == crate::element::Camp::Lacklandists
                    && !soldiers_with_lower_pride
                    && tick.friends_nearer_to_enemy
                        >= num_enemies_i_can_see as u16
                            + (num_enemies_i_can_see as f32 * 0.045 * self.get_courage() as f32)
                                as u16
                {
                    // Lacklandist observe — enough friends are already
                    // fighting closer to the enemy, stand back and watch.
                    // Camp-gated: only Lacklandists take this branch;
                    // royalists fall through to Fight.
                    // `num_enemies_i_can_see` is a persistent count of
                    // tracked enemies, not a per-tick "detected this
                    // frame" count, since `tick.personally_visible_enemies`
                    // is only populated on the detection-commit dispatch
                    // path; otherwise EVENT_TIMER-driven calls would see
                    // `0 >= 0 + 0 = true` and wrongly observe instead of
                    // charging.
                    decision = Decision::Observe;
                } else {
                    // Charge! (Earlier port versions injected a
                    // `refresh_arrow_protection` early-return here, but
                    // the offensive-decision chain does not call
                    // RefreshArrowProtection — that sweep lives in
                    // The16thFrame and a few explicit call sites.)
                    decision = Decision::Fight;
                }
            } else {
                // `only_enemy_soldiers` is initialized true, cleared if
                // any PC is in list_them. Used to gate LookForHelp /
                // RunAndAlertSoldiers — you don't call for help if your
                // opponents are all enemy soldiers (friendly fire / brawl
                // semantics).
                let only_enemy_soldiers = !self
                    .list_them
                    .iter()
                    .any(|&h| self.find_fighter(h, tick).map(|f| f.is_pc).unwrap_or(false));

                // Archer with no arrows → run for new arrows.
                if self.is_archer() && ctx.remaining_arrows == 0 {
                    decision = Decision::RunForNewArrows;
                } else {
                    match self.get_rank() {
                        ProfileRank::Soldier
                            if !self.base.friends_are_alerted
                                && !only_enemy_soldiers
                                && self.base.blood_alcohol == 0 =>
                        {
                            decision = Decision::LookForHelp;
                        }
                        ProfileRank::Soldier => {
                            decision = Decision::Cassos;
                        }
                        ProfileRank::Officer
                            if !self.base.friends_are_alerted
                                && !only_enemy_soldiers
                                && self.base.blood_alcohol == 0 =>
                        {
                            decision = Decision::RunAndAlertSoldiers;
                        }
                        ProfileRank::Officer => {
                            decision = Decision::Cassos;
                        }
                        _ => {
                            decision = Decision::Cassos;
                        }
                    }
                }
            }
        }

        tracing::trace!(
            me = self.base.me,
            ?decision,
            primary_target = self.base.primary_target,
            num_enemies_i_can_see,
            "battle_decisions: chose decision"
        );
        // Carry out decision (with possible fallback loop)
        self.execute_battle_decision(decision, cover_shield_bearer, global, ctx, tick, grid);

        self.base
            .register_log_line(LogLineType::BattleDecision, decision as u16);
    }

    /// Execute a battle decision, with fallback to alternative decisions if needed.
    /// `cover_shield_bearer` is the handle of the shield bearer chosen during the
    /// decision phase for `CoverBehindShieldBearer`; 0 for all other decisions.
    fn execute_battle_decision(
        &mut self,
        mut decision: Decision,
        cover_shield_bearer: HumanHandle,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Allow up to 5 fallback decision changes to prevent infinite loops
        for _ in 0..5 {
            match decision {
                Decision::Fight => {
                    let target = self.get_new_primary_target(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED,
                        ctx,
                        tick,
                    );
                    if target != 0 {
                        self.base.primary_target = target;
                        self.attack_enemy(target, Some(&mut *global), ctx, tick, grid);
                        if self.base.couldnt_reachpoint {
                            self.base.couldnt_reachpoint = false;
                            decision = Decision::Observe;
                            continue;
                        }
                    } else {
                        decision = Decision::Observe;
                        continue;
                    }
                }

                Decision::Reserve => {
                    let target = self.get_new_primary_target(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                    );
                    self.base.primary_target = target;
                    if target != 0 {
                        self.base.pending_focus = Some(target);
                    } else {
                        self.base.pending_unfocus = true;
                    }
                    self.set_state(AiState::Attacking, Substate::AttackingReserve);
                    self.base.launch_timer(50, ctx.frame);
                }

                Decision::LastReserve => {
                    let target = self.get_new_primary_target(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                    );
                    self.base.primary_target = target;
                    if ctx.self_action_state.is_sword() {
                        if crate::sim_rng::u32(0..4) == 0 {
                            self.base
                                .pending_launch_commands
                                .push(crate::element::Command::Provoke);
                        } else if let Some(target_pos) = self
                            .find_fighter(target, tick)
                            .map(|f| f.position)
                            .or_else(|| ctx.entity_view(target).map(|view| view.position))
                        {
                            let d = pos_diff(&target_pos, &ctx.position);
                            let dir = vec_to_sector(d.0, d.1);
                            self.base.pending_set_direction_instantly = Some(dir as i16);
                        }
                    } else {
                        self.base.pending_enter_swordfight =
                            Some(EnterSwordfightRequest::RaiseSword);
                        self.base.pending_enter_swordfight_jump_line = None;
                    }
                    if target != 0 {
                        self.base.pending_focus = Some(target);
                    } else {
                        self.base.pending_unfocus = true;
                    }
                    self.set_state(AiState::Attacking, Substate::AttackingLastReserve);
                    self.base.launch_timer(50, ctx.frame);
                }

                Decision::Observe => {
                    let target = self.get_new_primary_target(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                    );
                    self.base.primary_target = target;
                    if target != 0 {
                        self.base.pending_focus = Some(target);
                    } else {
                        self.base.pending_unfocus = true;
                    }
                    self.base.set_emoticon(EmoticonType::XMark);
                    if self.combat_trainer {
                        self.set_state(AiState::Attacking, Substate::AttackingApproachToObserve);
                        self.base.launch_timer(1, ctx.frame);
                    } else {
                        // DECISION_OBSERVE uses the swordfight-observer
                        // courage distance, not the proud-observer constant,
                        // and launches a 50-tick timer even while approaching
                        // so observers keep reconsidering if the active
                        // fighter drops or the formation changes.
                        if let Some(target_pos) = tick
                            .nearby_fighters
                            .iter()
                            .find(|f| f.handle == target)
                            .map(|f| f.position)
                        {
                            self.base.seek_position = target_pos;
                            let observe_distance = AiController::value_between(
                                parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
                                parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
                                self.get_courage() as u8,
                            );
                            self.go_near(
                                AiState::Attacking,
                                Substate::AttackingApproachToObserve,
                                target_pos,
                                observe_distance as i32,
                                GotoFlags::empty(),
                                ctx,
                            );
                            self.base.launch_timer(50, ctx.frame);
                        } else {
                            self.set_state(
                                AiState::Attacking,
                                Substate::AttackingApproachToObserve,
                            );
                            self.base.launch_timer(50, ctx.frame);
                        }
                    }
                }

                Decision::Shoot => {
                    if ctx.remaining_arrows == 0 {
                        decision = Decision::RunForNewArrows;
                        continue;
                    }
                    // Pick best shot target.
                    let target = self.propose_shot_target(ctx, tick);
                    if target != 0 {
                        self.base.primary_target = target;
                        self.base.pending_focus = Some(target);
                        // AIMING_TIME_FORMULA = (110 - shooting_ability) / 2.
                        // Use the soldier's modified shooting ability
                        // (with alcohol penalty) — *not* IQ — so the
                        // bow-aim timer tracks `shooting`.
                        if ctx.self_action_state.is_bow() {
                            if self.base.current_substate == Substate::AttackingBowAiming {
                                self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                                self.shoot_arrow_at(target, ctx, tick);
                            } else {
                                let aim_time = ((110u32)
                                    .saturating_sub(self.get_shooting_ability(ctx) as u32))
                                    / 2;
                                self.set_state(AiState::Attacking, Substate::AttackingBowAiming);
                                self.base.launch_timer(aim_time.max(5), ctx.frame);
                            }
                        } else {
                            self.base.stop_all();
                            self.set_state(AiState::Attacking, Substate::AttackingBowLoading);
                            self.base
                                .pending_launch_commands
                                .push(if self.enemy_seen_below {
                                    crate::element::Command::EquipBowDown
                                } else {
                                    crate::element::Command::EquipBow
                                });
                        }
                    } else {
                        // No valid target — fall back to observe
                        decision = Decision::ArcherObserve;
                        continue;
                    }
                }

                Decision::Cassos => {
                    // In Merry Man Forest, try to flee via
                    // MerryManForestCassos first. Otherwise: random
                    // Cassos/Panic remark, pick a primary target, then
                    // Panic(target_pos, AI_STANDARD_PANIC_RUNS) — note
                    // the threat point is the target's *current*
                    // position, NOT seek_position.
                    if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global)
                    {
                        if crate::sim_rng::u32(0..2) == 0 {
                            self.base.say(Remark::Cassos);
                        } else {
                            self.base.say(Remark::Panic);
                        }
                        let target = self.get_new_primary_target(
                            PrimaryTargetFlags::VIPS_ALLOWED,
                            ctx,
                            tick,
                        );
                        self.base.primary_target = target;
                        // Use live target position when available; fall
                        // back to seek_position only if the snapshot is
                        // empty.
                        let threat = self
                            .find_fighter(target, tick)
                            .map(|f| f.position)
                            .unwrap_or(self.base.seek_position);
                        self.panic_from_position(
                            threat,
                            parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
                        );
                    }
                }

                Decision::LookForHelp => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    // Center is the live target position, not
                    // seek_position.
                    let center = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .unwrap_or(self.base.seek_position);
                    // `bAlertingSoldierNear` short-circuits to Cassos: if
                    // another soldier is already running to alert an
                    // officer, don't dispatch a duplicate run.
                    let alerting_soldier_near = tick.camp_soldiers.iter().any(|cs| {
                        cs.handle != self.base.me
                            && cs.ai_substate == Substate::SeekingRunningToOfficer
                    });
                    if alerting_soldier_near || !self.alert_officer(center, 0, ctx, tick) {
                        decision = Decision::Cassos;
                        continue;
                    } else {
                        // Random Cassos/Panic remark.
                        if crate::sim_rng::u32(0..2) == 0 {
                            self.base.say(Remark::Cassos);
                        } else {
                            self.base.say(Remark::Panic);
                        }
                    }
                }

                Decision::AlertSoldiers => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    // DECISION_ALERT_SOLDIERS calls CommandSoldiersToAttack,
                    // NOT AlertSoldiers, with the live target position.
                    let center = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .unwrap_or(self.base.seek_position);
                    if !self.command_soldiers_to_attack(center, global, grid, ctx, tick) {
                        decision = Decision::Reserve;
                        continue;
                    } else {
                        self.base.say(Remark::OfficerGivesAttackOrder);
                    }
                }

                Decision::RunAndAlertSoldiers => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    // Use the primary target's last known position.
                    // Fall back to our own seek point if the target
                    // snapshot is missing.
                    let center = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .unwrap_or(self.base.seek_position);
                    if !self.run_and_alert_soldiers(center, ctx, tick, global) {
                        decision = Decision::Cassos;
                        continue;
                    } else {
                        // Random Cassos/Panic remark.
                        if crate::sim_rng::u32(0..2) == 0 {
                            self.base.say(Remark::Cassos);
                        } else {
                            self.base.say(Remark::Panic);
                        }
                    }
                }

                Decision::TowerGuardAlert => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    // Track the target we just locked in. Fall back to our
                    // own position when the fighter snapshot is missing
                    // so the soldier points somewhere sensible instead of
                    // the origin.
                    self.base.seek_position = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .or_else(|| ctx.entity_view(target).map(|view| view.position))
                        .unwrap_or(ctx.position);
                    self.set_state(AiState::Attacking, Substate::AttackingTowerGuardAlert);
                    self.base.point_to(self.base.seek_position);
                }

                Decision::TowerGuardObserve => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    self.base.seek_position = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .or_else(|| ctx.entity_view(target).map(|view| view.position))
                        .unwrap_or(ctx.position);
                    self.set_state(AiState::Attacking, Substate::AttackingTowerGuardObserve);
                    self.base.face_entity(target, ctx);
                    self.base.launch_timer(100, ctx.frame);
                }

                Decision::RunForNewArrows => {
                    // Find nearest door with arrow reserves and run to it.
                    self.base.say(Remark::OutOfAmmunition);

                    // Remember target's position so the archer can sprint
                    // back toward where the fight was after picking up
                    // arrows. Writes unconditionally when
                    // `primary_target != 0`; mirror that by falling back
                    // to the entity view when the target isn't in the
                    // per-tick fighter snapshot (off-grid /
                    // dead-but-not-cleared / out of proximity range), so
                    // we never leave a stale seek_position from a
                    // previous state.
                    if self.base.primary_target != 0 {
                        let target_pos = tick
                            .nearby_fighters
                            .iter()
                            .find(|f| f.handle == self.base.primary_target)
                            .map(|f| f.position)
                            .or_else(|| {
                                ctx.entity_view(self.base.primary_target)
                                    .map(|v| v.position)
                            });
                        if let Some(p) = target_pos {
                            self.base.seek_position = p;
                        }
                    } else {
                        self.base.seek_position = ctx.position;
                    }

                    // GetNearestDoor(NULL, true) port. Same filter chain
                    // as the civilian Panic flee: building doors only,
                    // authorized for this NPC, skip the actor's own
                    // building, distance by `MaxNorm` with +500
                    // sector-change / +300 layer-change malus. The
                    // `arrow_reserves=true` arg adds the per-house
                    // `HasArrowReserve` predicate (read from
                    // `House::arrow_reserve`, loaded at level time from
                    // the GUYS/CAVE tenant chunk). The `dangerous_house`
                    // check is Lacklandist-only; the archer
                    // RunForNewArrows path fires on Royalists, so the
                    // gate is inert here — but we still mirror the camp
                    // guard for correctness if a modded level ever runs
                    // a Lacklandist archer.
                    // PC-in-house checks are represented through the
                    // shared house/door snapshot available on `global`.
                    let my_building_num: Option<u16> = ctx
                        .in_building
                        .then_some(ctx.building_sector)
                        .flatten()
                        .map(u16::from);
                    let my_sector_num: Option<u16> = ctx.position.sector.map(u16::from);
                    let my_layer = ctx.position.level;
                    let nearest_door_pos = {
                        let mut best: Option<(crate::ai::Position, u32)> = None;
                        for door in global.door_seek_infos.iter() {
                            if !matches!(door.door_type, crate::gate::DoorType::Building) {
                                continue;
                            }
                            if !door.npc_villain_authorized_direct {
                                continue;
                            }
                            if my_building_num == Some(door.sector_in) {
                                continue;
                            }
                            // Arrow-reserve filter.
                            let has_reserve = global
                                .houses
                                .iter()
                                .find(|h| h.sector_index == door.sector_in as u32)
                                .map(|h| h.arrow_reserve)
                                .unwrap_or(false);
                            if !has_reserve {
                                continue;
                            }
                            let dx = (door.point_out.0 - ctx.position.x).abs();
                            let dy = (door.point_out.1 - ctx.position.y).abs();
                            let mut distance = dx.max(dy) as u32;
                            if Some(door.sector_out) != my_sector_num {
                                distance = distance.saturating_add(500);
                            }
                            if door.layer_out != my_layer {
                                distance = distance.saturating_add(300);
                            }
                            if best.map(|(_, d)| distance < d).unwrap_or(true) {
                                best = Some((door.position_in, distance));
                            }
                        }
                        best.map(|(p, _)| p)
                    };

                    if let Some(door_pos) = nearest_door_pos {
                        self.base
                            .set_transient_emoticon(EmoticonType::XMark, 100, 0);
                        self.go_to(
                            AiState::Fleeing,
                            Substate::FleeingRunForArrowReserves,
                            door_pos,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // No door found — fall back to flee
                        decision = Decision::Cassos;
                        continue;
                    }
                }

                Decision::TooProudToAttack => {
                    // Stand back and observe from a comfortable distance
                    // while lesser soldiers fight.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    let target_pos = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .unwrap_or(ctx.position);
                    let d = pos_diff(&target_pos, &ctx.position);
                    let distance = iso_norm(d, ASPECT_RATIO);

                    if distance < parameters_ai::PROUD_OBSERVER_MIN_DISTANCE as f32 {
                        // Too close — step back.
                        if let Some(goal) = self.propose_good_step_back_goal(
                            target_pos,
                            parameters_ai::PROUD_OBSERVER_GOOD_DISTANCE,
                            parameters_ai::PROUD_OBSERVER_MIN_DISTANCE,
                            ctx,
                            grid,
                            ASPECT_RATIO,
                        ) {
                            self.go_to(
                                AiState::Attacking,
                                Substate::AttackingTooProudToAttackRetire,
                                goal,
                                GotoFlags::empty(),
                                ctx,
                            );
                        } else {
                            // Can't retreat — fight instead.
                            decision = Decision::Fight;
                            continue;
                        }
                    } else if distance > parameters_ai::PROUD_OBSERVER_MAX_DISTANCE as f32 {
                        // Too far — approach.
                        self.go_near(
                            AiState::Attacking,
                            Substate::AttackingTooProudToAttackApproach,
                            target_pos,
                            parameters_ai::PROUD_OBSERVER_GOOD_DISTANCE as i32,
                            GotoFlags::empty(),
                            ctx,
                        );
                        if self.base.already_on_point {
                            self.base.already_on_point = false;
                            let dir = vec_to_sector(d.0, d.1);
                            self.base.pending_set_direction_instantly = Some(dir as i16);
                            self.set_state(AiState::Attacking, Substate::AttackingTooProudToAttack);
                            self.base.launch_timer(20, ctx.frame);
                        }
                    } else {
                        // Good distance — face and observe.
                        let dir = vec_to_sector(d.0, d.1);
                        self.base.pending_set_direction_instantly = Some(dir as i16);
                        self.base.pending_focus = Some(self.base.primary_target);
                        self.set_state(AiState::Attacking, Substate::AttackingTooProudToAttack);
                        self.base.launch_timer(20, ctx.frame);
                    }

                    // Only on first battle decision entry.
                    if self.previous_substate == Substate::AttackingReactiontime
                        || self.previous_substate == Substate::AttackingReactiontimeRunning
                    {
                        if self.is_vip {
                            self.base.say(Remark::VipProudDontFight);
                        } else {
                            self.base.say(Remark::ProudDontFight);
                        }
                    }
                }

                Decision::ArcherStepBack => {
                    // Archer steps back from enemy that's too close, then
                    // re-evaluates.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    let enemy_pos = self
                        .find_fighter(target, tick)
                        .map(|f| f.position)
                        .unwrap_or(ctx.position);
                    self.base.seek_position = enemy_pos;
                    if let Some(goal) = self.propose_good_step_back_goal(
                        enemy_pos,
                        parameters_ai::ARCHER_GOOD_DISTANCE,
                        parameters_ai::ARCHER_MIN_DISTANCE,
                        ctx,
                        grid,
                        ASPECT_RATIO,
                    ) {
                        self.go_to(
                            AiState::Attacking,
                            Substate::AttackingArcherRetireFromCombat,
                            goal,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // Can't step back — fall back to shooting.
                        decision = Decision::Shoot;
                        continue;
                    }
                }

                Decision::ArcherObserve => {
                    let target = self.get_new_primary_target(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                    );
                    self.base.primary_target = target;
                    if target != 0 {
                        self.base.pending_focus = Some(target);
                    } else {
                        self.base.pending_unfocus = true;
                    }

                    if ctx.self_action_state.is_bow() {
                        self.set_state(AiState::Attacking, Substate::AttackingBowObserving);
                        self.base.launch_timer(50, ctx.frame);
                    } else {
                        self.base.stop_all();
                        self.base
                            .pending_launch_commands
                            .push(if self.enemy_seen_below {
                                crate::element::Command::EquipBowDown
                            } else {
                                crate::element::Command::EquipBow
                            });
                        self.set_state(AiState::Attacking, Substate::AttackingBowObservingLoading);
                    }
                }

                Decision::Menace => {
                    // Menace a PC in coma.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.set_state(AiState::Menacing, Substate::MenacingPcInComa);
                    self.base
                        .launch_timer(parameters_ai::AI_MENACING_PATIENCE as u32, ctx.frame);
                }

                Decision::CoverBehindShieldBearer => {
                    // Run to cover position behind shield bearer.
                    self.update_shield_bearer_before_me(cover_shield_bearer);
                    // Adopt the shield bearer's primary target.
                    if let Some(sb_snap) = self.find_fighter(cover_shield_bearer, tick) {
                        self.base.primary_target = sb_snap.primary_target;
                    }
                    if let Some(cover_pos) = self.compute_position_behind_shield_bearer(
                        self.shield_bearer_before_me,
                        ctx,
                        tick,
                        grid,
                    ) {
                        // Cover point must be within view radius of the
                        // primary target, otherwise the archer can't see
                        // the enemy from behind the shield bearer.
                        let target_pos = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.position)
                            .unwrap_or(ctx.position);
                        let d = pos_diff(&target_pos, &cover_pos);
                        if square_norm(d) >= ctx.sq_standard_view_radius {
                            // Cover point too far from target — fall back to shoot
                            self.update_shield_bearer_before_me(0);
                            decision = Decision::Shoot;
                            continue;
                        }

                        self.base.seek_position = cover_pos;
                        self.go_to(
                            AiState::Attacking,
                            Substate::AttackingBowRunningBehindShieldBearer,
                            cover_pos,
                            GotoFlags::RUN,
                            ctx,
                        );

                        if self.base.already_on_point {
                            // Already in position — check facing
                            let target_pos = self
                                .find_fighter(self.base.primary_target, tick)
                                .map(|f| f.position)
                                .unwrap_or(cover_pos);
                            let dx = target_pos.x - ctx.position.x;
                            let dy = target_pos.y - ctx.position.y;
                            let desired_dir = vec_to_sector(dx, dy);
                            if ctx.direction == desired_dir {
                                self.base.already_on_point = false;
                                decision = Decision::Shoot;
                                continue;
                            }
                        }
                        // Tell the shield bearer to announce the formation.
                        self.base
                            .pending_cross_npc_actions
                            .push(CrossNpcAction::Say {
                                target: cover_shield_bearer,
                                remark: Remark::ArchersBehindShieldBearers,
                            });
                    } else {
                        // Can't compute position — give up cover attempt.
                        self.update_shield_bearer_before_me(0);
                        decision = Decision::Shoot;
                        continue;
                    }
                }

                Decision::RunToArcheryPoint => {
                    // Run to the next waypoint on the archery path.
                    if let Some(wp) = self.archery_path_get_waypoint(global) {
                        // Remember enemy elevation for later bend decision
                        self.enemy_had_this_elevation = self
                            .find_fighter(self.base.primary_target, tick)
                            .map(|f| f.elevation)
                            .unwrap_or(0);
                        if wp.is_shooting_point {
                            // Run directly to shooting point (final
                            // sprint). SetMyShootingPoint writes the
                            // owner back so other archers scanning
                            // `pt.owner.is_none()` see the point as
                            // reserved.
                            if let Some(sec_idx) = self.my_archery_sector {
                                let pt_idx = u16::from(self.my_archery_point_index);
                                self.set_my_shooting_point(global, Some((sec_idx, pt_idx)));
                            }
                            self.go_to(
                                AiState::Attacking,
                                Substate::AttackingArcherRunOnShootingPathFinalSprint,
                                wp.position,
                                GotoFlags::RUN,
                                ctx,
                            );
                        } else {
                            // Run to first waypoint on path
                            self.go_to(
                                AiState::Attacking,
                                Substate::AttackingArcherRunOnShootingPath,
                                wp.position,
                                GotoFlags::RUN | GotoFlags::DONT_STOP,
                                ctx,
                            );
                        }
                    } else {
                        // Something went wrong — fall back to shoot
                        decision = Decision::Shoot;
                        continue;
                    }
                }

                _ => {
                    // Fallback — just fight
                    decision = Decision::Fight;
                    continue;
                }
            }
            break; // Decision executed successfully
        }
    }

    // -----------------------------------------------------------------------
    // AttackEnemy — engage an enemy
    // -----------------------------------------------------------------------

    pub(super) fn attack_enemy(
        &mut self,
        enemy: HumanHandle,
        global: Option<&mut AiGlobalState>,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Rider charge wins before any state is committed. Run the charge
        // attempt first and early-return; only if it bails do we mutate
        // primary_target / seek_position / emoticon. Otherwise a
        // successful charge would leave the soldier with an X-mark
        // emoticon and a primary_target the reference never sets here.
        if ctx.self_is_rider && self.maybe_make_rider_attack(ctx, tick, grid) {
            return;
        }

        // Unconditional seek_position = Position(enemy). Prefer the
        // per-tick fighter snapshot, but fall back to the engine entity
        // view so we never re-use a stale `seek_position` from a
        // previous state.
        let enemy_pos = tick
            .nearby_fighters
            .iter()
            .find(|f| f.handle == enemy)
            .map(|f| f.position)
            .or_else(|| ctx.entity_view(enemy).map(|v| v.position));
        if let Some(p) = enemy_pos {
            self.base.seek_position = p;
        }

        // primary_target then emoticon.
        self.base.primary_target = enemy;
        if let Some(global) = global
            && !global
                .same_frame_target_claims
                .iter()
                .any(|&(attacker, target)| attacker == self.base.me && target == enemy)
        {
            global.same_frame_target_claims.push((self.base.me, enemy));
        }
        debug_assert!(
            ctx.entity_view(enemy)
                .map(|v| v.camp != ctx.camp)
                .unwrap_or(true),
            "attack_enemy: target is a friend",
        );
        self.base.set_emoticon(EmoticonType::XMark);

        // Compute distance from `seek_position` (which is now fresh).
        let distance = {
            let dx = ctx.position.x - self.base.seek_position.x;
            let dy = ctx.position.y - self.base.seek_position.y;
            (dx * dx + dy * dy).sqrt()
        };
        self.reconsider_enemy_approach(false, distance, ctx, tick, grid);
    }

    // -----------------------------------------------------------------------
    // ReconsiderEnemyApproach — approach logic for melee
    // Simplified port of RHArtificialMalignity::ReconsiderEnemyApproach
    // -----------------------------------------------------------------------

    /// Decide how to approach the primary target: run when far, walk
    /// when close, fight when in melee range.
    ///
    /// `distance` is the world-distance from self to the primary target
    /// (caller computes it because the AI struct doesn't own a position).
    /// `seek_position` must already be set to the target's position
    /// before calling.
    ///
    /// Rider charge is handled by `maybe_make_rider_attack` (called
    /// from `attack_enemy`). Line-jump data is precomputed by the engine
    /// in `AiPerTickData::primary_target_jump_line`.
    pub fn reconsider_enemy_approach(
        &mut self,
        reachpoint: bool,
        _distance_arg: f32,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let standard_sword_range = self.sword_range as f32;
        // sword_range = standard sword range + 10.
        let sword_range: f32 = standard_sword_range + 10.0;
        let mut run_distance = self.compute_enemy_run_distance() as f32;

        // Already swordfighting? stay.
        if ctx.is_swordfighting {
            self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
            self.base.launch_timer(30, ctx.frame);
            return;
        }

        // Arrow-protection branch claims the decision.
        if self.refresh_arrow_protection(false, ctx, tick, grid) {
            return;
        }

        let mut b_reconsider = false;

        // Target on another entity's shoulders: re-point `primary_target`
        // to the carrier so every downstream read (friend-swap
        // comparison, `Focus`, `BeginSwordfight`'s
        // `pending_enter_swordfight`) sees the carrier rather than the
        // carried entity. The reference does
        // `primary_target = primary_target.GetCarrier()`, persisting
        // across ticks because `primary_target` is a member.
        let target_on_shoulders = matches!(
            tick.primary_target_posture,
            Some(crate::element::Posture::OnShoulders)
        );
        if target_on_shoulders && let Some(carrier_handle) = tick.primary_target_carrier_handle {
            self.base.primary_target = carrier_handle;
        }

        // Position(primary_target) after the substitution resolves to
        // the carrier's position when the carry path fired.
        let live_target_pos =
            if target_on_shoulders && let Some(carrier) = tick.primary_target_carrier_position {
                carrier
            } else {
                tick.primary_target_position
                    .unwrap_or(self.base.seek_position)
            };

        // Live distance from me to the target.
        let distance = {
            let dx = live_target_pos.x - ctx.position.x;
            let dy = (live_target_pos.y - ctx.position.y)
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            (dx * dx + dy * dy).sqrt()
        };

        // Pre-computed line-jump for table swordfight.
        let my_line_jump = tick.primary_target_jump_line;

        // Target-swap with a same-camp friend if the swap shortens the
        // total travel distance. `friend_swap_candidates` is the
        // engine's enumeration of same-camp soldiers currently
        // approaching an enemy; we walk them in enumeration order and
        // commit the first strict improvement.
        let mut working_target = self.base.primary_target;
        let mut working_target_pos = live_target_pos;
        let mut working_distance = distance;
        let mut swap_action: Option<(crate::ai::HumanHandle, crate::ai::HumanHandle)> = None;
        // Iterate friends only when we have our own target —
        // Position(primary_target) would crash on NULL otherwise. Skip
        // the swap heuristic if our primary_target is unset so we never
        // hand 0 to a friend via `pending_friend_primary_target_swap`.
        for cand in &tick.friend_swap_candidates {
            if working_target == 0 {
                break;
            }
            if cand.friend_primary_target == working_target {
                continue;
            }
            let me_to_friend_target = {
                let dx = ctx.position.x - cand.friend_primary_target_position.x;
                let dy = ctx.position.y - cand.friend_primary_target_position.y;
                (dx * dx + dy * dy).sqrt()
            };
            let friend_to_my_target = {
                let dx = cand.friend_position.x - working_target_pos.x;
                let dy = cand.friend_position.y - working_target_pos.y;
                (dx * dx + dy * dy).sqrt()
            };
            let friend_to_friend_target = {
                let dx = cand.friend_position.x - cand.friend_primary_target_position.x;
                let dy = cand.friend_position.y - cand.friend_primary_target_position.y;
                (dx * dx + dy * dy).sqrt()
            };
            if me_to_friend_target + friend_to_my_target
                < working_distance + friend_to_friend_target
            {
                swap_action = Some((cand.friend_handle, working_target));
                working_target = cand.friend_primary_target;
                working_target_pos = cand.friend_primary_target_position;
                working_distance = me_to_friend_target;
            }
        }
        if let Some((friend, new_tgt)) = swap_action {
            self.base.primary_target = working_target;
            self.base.pending_friend_primary_target_swap = Some((friend, new_tgt));
        }

        // Primary target is in a non-stairs lift: run to the entry
        // point matching the evaluating NPC's layer.
        if tick.primary_target_in_lift
            && let Some(entry) = tick.primary_target_lift_entry
        {
            self.base.pending_focus = Some(working_target);
            self.base.seek_position = entry;
            self.go_near(
                AiState::Attacking,
                Substate::AttackingRunningToLadder,
                entry,
                30,
                GotoFlags::RUN,
                ctx,
            );
            self.base.launch_timer(30, ctx.frame);
            return;
        }

        // Substate-derived charge / first_consideration flags.
        let (mut b_charge, b_first_consideration) = match self.base.current_substate {
            Substate::AttackingRunningToEnemy | Substate::AttackingWalkingToEnemy => (false, false),
            Substate::AttackingChargingEnemy => {
                if my_line_jump.is_none() && !tick.primary_target_in_lift {
                    (true, false)
                } else {
                    b_reconsider = true;
                    (false, false)
                }
            }
            Substate::AttackingReactiontime | Substate::AttackingReactiontimeRunning => {
                let mut c = self.sword_is_charge_weapon;
                c &= self.get_courage() >= crate::ai_enemy::combat::CHARGE_MIN_COURAGE;
                c &= (working_distance as i32) >= crate::ai_enemy::combat::CHARGE_MIN_DISTANCE;
                c &= my_line_jump.is_none();
                c &= !ctx.self_is_rider;
                c &= !tick.primary_target_in_lift;
                if c {
                    self.base.say(crate::ai::Remark::Warcry);
                }
                (c, true)
            }
            _ => (false, true),
        };

        // Lock eye-tracking onto the primary target.
        self.base.pending_focus = Some(working_target);

        // Riders try charge attack first.
        if ctx.self_is_rider && self.maybe_make_rider_attack(ctx, tick, grid) {
            return;
        }

        // Close enough to fight? Charging units defer until the
        // reachpoint has been hit; everyone else engages immediately.
        if working_distance <= sword_range && (!b_charge || reachpoint) {
            self.begin_swordfight(ctx, tick);
            return;
        }

        // First-consideration / reachpoint force reconsider.
        b_reconsider = b_reconsider || reachpoint || b_first_consideration;

        let mut b_below_run_distance;
        if b_charge {
            // Charge: 10 sq-norm target-moved threshold.
            let target_moved = {
                let dx = working_target_pos.x - self.base.seek_position.x;
                let dy = working_target_pos.y - self.base.seek_position.y;
                dx * dx + dy * dy > 10.0
            };
            b_reconsider = b_reconsider || (target_moved && !self.pc_missed);
            b_below_run_distance = false;
        } else {
            // Normal: 100 sq-norm target-moved threshold.
            let target_moved = {
                let dx = working_target_pos.x - self.base.seek_position.x;
                let dy = working_target_pos.y - self.base.seek_position.y;
                dx * dx + dy * dy > 100.0
            };
            b_reconsider = b_reconsider || (target_moved && !self.pc_missed);
            // Drop to walk once already running + near enough.
            b_below_run_distance = working_distance < (run_distance + 10.0);
            b_reconsider = b_reconsider
                || (self.base.current_substate == Substate::AttackingRunningToEnemy
                    && b_below_run_distance);
        }

        // Riders always run.
        b_below_run_distance &= !ctx.self_is_rider;

        // "A walking circus pyramid!" override.
        // The comparison is literally `GetCommand() != WalkingCarryingOnShoulders`,
        // so it's true for every normal target. Effect: drop charge +
        // below-run-distance, force reconsider, shrink run distance to
        // plain sword range. The carry-on-shoulders branch is the
        // quiet path where charge / close-walk are preserved.
        if !matches!(
            tick.primary_target_animation,
            Some(crate::order::OrderType::WalkingCarryingOnShoulders)
        ) {
            b_charge = false;
            b_below_run_distance = false;
            b_reconsider = true;
            run_distance = standard_sword_range;
        }

        if !b_reconsider {
            self.base.launch_timer(10, ctx.frame);
            return;
        }

        // Commit new seek goal.
        let mut pos_prim_target = working_target_pos;
        if let Some(line_idx) = my_line_jump
            && let Some(g) = grid
            && let Some(on_line) = self.compute_jump_line_target(g, line_idx, pos_prim_target)
        {
            pos_prim_target = on_line;
        }
        self.base.seek_position = pos_prim_target;

        // Re-focus (redundant but mirrored for parity).
        self.base.pending_focus = Some(working_target);

        // Not below run distance: charge or run.
        if !b_below_run_distance {
            if b_charge {
                // GoNear(target, sword_range, RUN | CHARGE).
                self.go_near(
                    AiState::Attacking,
                    Substate::AttackingChargingEnemy,
                    pos_prim_target,
                    standard_sword_range as i32,
                    GotoFlags::RUN | GotoFlags::CHARGE,
                    ctx,
                );
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.begin_swordfight(ctx, tick);
                    return;
                }
                self.base.launch_timer(10, ctx.frame);
            } else {
                // GoNear(target, run_distance, RUN | DONT_STOP).
                self.go_near(
                    AiState::Attacking,
                    Substate::AttackingRunningToEnemy,
                    pos_prim_target,
                    run_distance as i32,
                    GotoFlags::RUN | GotoFlags::DONT_STOP,
                    ctx,
                );
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.begin_swordfight(ctx, tick);
                    return;
                }
                self.base.launch_timer(10, ctx.frame);
            }
        } else {
            // Below run distance: walk, or run if target is running.
            let target_is_running = matches!(
                tick.primary_target_animation,
                Some(crate::order::OrderType::RunningUpright)
            );
            if target_is_running {
                if my_line_jump.is_none() {
                    // GoNear(target, sword_range, RUN | DONT_STOP).
                    self.go_near(
                        self.base.current_state,
                        self.base.current_substate,
                        pos_prim_target,
                        standard_sword_range as i32,
                        GotoFlags::RUN | GotoFlags::DONT_STOP,
                        ctx,
                    );
                } else {
                    // GoTo(target, RUN | DONT_STOP).
                    self.base
                        .go_to(pos_prim_target, GotoFlags::RUN | GotoFlags::DONT_STOP, ctx);
                }
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.begin_swordfight(ctx, tick);
                    return;
                }
                self.set_state(AiState::Attacking, Substate::AttackingRunningToEnemy);
                self.base.launch_timer(10, ctx.frame);
            } else {
                if my_line_jump.is_none() {
                    // GoNear(target, sword_range, 0) — walk.
                    self.go_near(
                        self.base.current_state,
                        self.base.current_substate,
                        pos_prim_target,
                        standard_sword_range as i32,
                        GotoFlags::empty(),
                        ctx,
                    );
                } else {
                    // GoTo(target) — walk on jump line.
                    self.go_to(
                        self.base.current_state,
                        self.base.current_substate,
                        pos_prim_target,
                        GotoFlags::empty(),
                        ctx,
                    );
                }
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.begin_swordfight(ctx, tick);
                    return;
                }
                self.set_state(AiState::Attacking, Substate::AttackingWalkingToEnemy);
                self.base.launch_timer(10, ctx.frame);
            }
        }

        // Couldn't-reachpoint avenger-on-roof fallback.
        // The engine tick pre-computes the blocking-gate wait position
        // into `tick.avenger_on_roof_wait_position` via
        // `compute_avenger_wait_position`; see engine/ai.rs.
        if self.base.couldnt_reachpoint
            && let Some(wait_pos) = tick.avenger_on_roof_wait_position
        {
            self.base.couldnt_reachpoint = false;
            self.go_near(
                AiState::Attacking,
                Substate::AttackingRunToAvengerOnRoof,
                wait_pos,
                50,
                GotoFlags::RUN,
                ctx,
            );
            self.base.launch_timer(30, ctx.frame);
        }
    }

    /// Compute the approach point on `line_idx` closest to the victim.
    /// Returns the point on the aggressor's jump-line B-end mirrored
    /// from the victim's nearest-point projection on the paired line.
    fn compute_jump_line_target(
        &self,
        grid: &crate::fast_find_grid::FastFindGrid,
        line_idx: u32,
        victim_pos: crate::ai::Position,
    ) -> Option<crate::ai::Position> {
        let aggressor_line = grid.level.jump_lines.get(line_idx as usize)?;
        let victim_line_idx = aggressor_line.associated_line_index?;
        let victim_line = grid.level.jump_lines.get(victim_line_idx as usize)?;
        let t_victim = victim_line
            .compute_nearest_point_param(crate::geo2d::pt(victim_pos.x, victim_pos.y).into());
        let coeff = t_victim * victim_line.norm();
        let aggressor_vec = aggressor_line.vector();
        let aggressor_len = aggressor_line.norm().max(f32::EPSILON);
        let inv_len = 1.0 / aggressor_len;
        Some(crate::ai::Position {
            x: aggressor_line.point_b.x - coeff * aggressor_vec.x * inv_len,
            y: aggressor_line.point_b.y - coeff * aggressor_vec.y * inv_len,
            sector: aggressor_line
                .sector_index
                .and_then(|s| SectorHandle::new(u32::from(s) as u16))
                .or(victim_pos.sector),
            level: aggressor_line.layer,
        })
    }

    // -----------------------------------------------------------------------
    // Rider combat — charge attack logic
    // -----------------------------------------------------------------------

    // Rider charge constants.
    const RIDER_CHARGE_LATERAL_DISTANCE: f32 = 40.0;
    const RIDER_CHARGE_SQR_LATERAL_DISTANCE: f32 = 1600.0;
    const RIDER_CHARGE_LOOP_DISTANCE: f32 = 80.0;
    const RIDER_CHARGE_SQR_LOOP_DISTANCE: f32 = 6400.0;
    const RIDER_CHARGE_MAX_LATERAL_DISTANCE: f32 = 65.0;
    const RIDER_MAX_REATTACK_DISTANCE: f32 = 500.0;

    /// Try to initiate a rider charge attack against any visible enemy.
    ///
    /// Returns `true` if a charge was initiated, `false` otherwise.
    pub fn maybe_make_rider_attack(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        assert!(ctx.self_is_rider);

        let my_pos = ctx.position;
        let my_dir = ctx.direction;

        // Try primary target first.
        let mut target = self.base.primary_target;
        let mut dest = Position::default();
        let mut begin_charge = false;
        let mut ok = false;

        // Find primary target position from fighter snapshots.
        let target_pos = tick
            .nearby_fighters
            .iter()
            .find(|f| f.handle == target && !f.is_friendly)
            .map(|f| f.position);

        if target != 0
            && let Some(tpos) = target_pos
        {
            // The reference checks !IsDead && !IsUnconscious && !IsTied.
            // `is_able_to_fight` already covers the first two; the
            // explicit `is_tied` check separates the bound posture
            // (where the engine still has the entity active so
            // `is_able_to_fight` could return true).
            let target_alive = tick
                .nearby_fighters
                .iter()
                .find(|f| f.handle == target)
                .map(|f| f.is_able_to_fight && !f.is_tied)
                .unwrap_or(false);

            if target_alive
                && let Some((d, bc)) = self.get_good_rider_attack_destination(
                    my_pos,
                    my_dir,
                    tpos,
                    ctx,
                    grid,
                    &tick.nearby_fighters,
                )
            {
                dest = d;
                begin_charge = bc;
                ok = true;
            }
        }

        // If primary target is unreachable, scan other enemies.
        if !ok {
            for enemy in &self.list_them {
                if *enemy == target {
                    continue;
                }
                let epos = match tick
                    .nearby_fighters
                    .iter()
                    .find(|f| f.handle == *enemy && !f.is_friendly && f.is_able_to_fight)
                    .map(|f| f.position)
                {
                    Some(p) => p,
                    None => continue,
                };
                if let Some((d, bc)) = self.get_good_rider_attack_destination(
                    my_pos,
                    my_dir,
                    epos,
                    ctx,
                    grid,
                    &tick.nearby_fighters,
                ) {
                    target = *enemy;
                    self.base.primary_target = target;
                    dest = d;
                    begin_charge = bc;
                    ok = true;
                    break;
                }
            }
        }

        if !ok {
            return false;
        }

        // seek_position = Position(primary_target).
        if let Some(tpos) = tick
            .nearby_fighters
            .iter()
            .find(|f| f.handle == target)
            .map(|f| f.position)
        {
            self.base.seek_position = tpos;
        }

        if !begin_charge {
            // Approach phase — ride toward enemy.
            self.set_state(
                AiState::Attacking,
                Substate::AttackingRiderChargingApproaching,
            );
            self.base
                .go_to(dest, GotoFlags::RUN | GotoFlags::RIDER_CHARGE, ctx);
        } else {
            // Close enough to charge — begin charge pass. Drop stare
            // lock so the rider's cone follows the charge direction, not
            // the fleeing target.
            self.base.pending_unfocus = true;
            self.base.say(crate::ai::Remark::Warcry);
            self.go_to(
                AiState::Attacking,
                Substate::AttackingRiderChargingPassing,
                dest,
                GotoFlags::RUN | GotoFlags::RIDER_CHARGE | GotoFlags::RIDER_CHARGE_HIT,
                ctx,
            );
        }

        true
    }

    /// Compute the charge destination for a rider attacking a specific enemy.
    ///
    /// The rider charges past the enemy at a lateral offset, so the hit zone
    /// polygon sweeps across the enemy. Returns `(destination, begin_charge_anim)`.
    fn get_good_rider_attack_destination(
        &self,
        my_pos: Position,
        my_dir: u16,
        enemy_pos: Position,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        nearby_fighters: &[FighterSnapshot],
    ) -> Option<(Position, bool)> {
        // Get vectors
        let nose = sector_to_vector_iso(my_dir, ASPECT_RATIO);

        // Vector to enemy (with Y stretched by inverse aspect ratio for isometric)
        let me_to_enemy_sy = (
            enemy_pos.x - my_pos.x,
            (enemy_pos.y - my_pos.y) * INVERSE_ASPECT_RATIO,
        );

        // Is the enemy in front of us?
        let nose_sy = (nose.0, nose.1 * INVERSE_ASPECT_RATIO);
        if dot2(nose_sy, me_to_enemy_sy) < 0.0 {
            return None;
        }

        // Compute distance.
        let sq_norm = me_to_enemy_sy.0 * me_to_enemy_sy.0 + me_to_enemy_sy.1 * me_to_enemy_sy.1;
        let norm = sq_norm.sqrt();

        if norm < Self::RIDER_CHARGE_LATERAL_DISTANCE {
            return None;
        }

        // Compute cos(alpha) — angle we must ride to pass at lateral offset
        let cos_alpha = (1.0 - Self::RIDER_CHARGE_SQR_LATERAL_DISTANCE / sq_norm).sqrt();

        // Compute orthogonal vector (perpendicular to me→enemy)
        // GetNormal(false) with AR=1 yields (mY, -mX) — 90° clockwise.
        let ortho = (me_to_enemy_sy.1, -me_to_enemy_sy.0);
        let ortho_len = (ortho.0 * ortho.0 + ortho.1 * ortho.1).sqrt();
        if ortho_len < f32::EPSILON {
            return None;
        }
        let ortho_norm = (ortho.0 / ortho_len, ortho.1 / ortho_len);
        let ortho_scaled = (
            ortho_norm.0 * Self::RIDER_CHARGE_LATERAL_DISTANCE / cos_alpha,
            ortho_norm.1 * Self::RIDER_CHARGE_LATERAL_DISTANCE / cos_alpha,
        );

        // Compute vector to hit point.
        let hit_point_sy = (
            me_to_enemy_sy.0 + ortho_scaled.0,
            me_to_enemy_sy.1 + ortho_scaled.1,
        );
        let hp_len = (hit_point_sy.0 * hit_point_sy.0 + hit_point_sy.1 * hit_point_sy.1).sqrt();
        if hp_len < f32::EPSILON {
            return None;
        }
        let hp_norm = (hit_point_sy.0 / hp_len, hit_point_sy.1 / hp_len);
        let hp_scaled = (hp_norm.0 * cos_alpha * norm, hp_norm.1 * cos_alpha * norm);

        // Reapply aspect ratio.
        let me_to_hit = (hp_scaled.0, hp_scaled.1 * ASPECT_RATIO);

        // Compute goal = hit point + LOOP_DISTANCE forward.
        let hit_norm_len = (me_to_hit.0 * me_to_hit.0 + me_to_hit.1 * me_to_hit.1).sqrt();
        if hit_norm_len < f32::EPSILON {
            return None;
        }
        let hit_dir = (me_to_hit.0 / hit_norm_len, me_to_hit.1 / hit_norm_len);
        let goal_x = my_pos.x + me_to_hit.0 + hit_dir.0 * Self::RIDER_CHARGE_LOOP_DISTANCE;
        let goal_y = my_pos.y + me_to_hit.1 + hit_dir.1 * Self::RIDER_CHARGE_LOOP_DISTANCE;

        // Check if straight movement from me to goal is clear.
        if let Some(g) = grid {
            let pt_me = crate::geo2d::pt(my_pos.x, my_pos.y);
            let pt_goal = crate::geo2d::pt(goal_x, goal_y);
            if !g.is_straight_movement_authorized(
                pt_me.into(),
                pt_goal.into(),
                my_pos.level,
                &ctx.move_box,
            ) {
                return None;
            }
        }

        // Check if charge would hit friendlies.
        // Build the strike zone polygon (4 corners of the charge sweep).
        {
            let me_to_hit_norm = hit_norm_len;
            // How far before the hit point does the strike begin?
            let mut strike_begins_before = me_to_hit_norm;
            while strike_begins_before > Self::RIDER_CHARGE_LOOP_DISTANCE {
                strike_begins_before -= Self::RIDER_CHARGE_LOOP_DISTANCE;
            }
            let dir_norm = (hit_dir.0, hit_dir.1);
            // GetNormal(true, ASPECT_RATIO) — (-mY / AR, mX * AR). Does NOT
            // re-normalize the result before scaling by RIDER_CHARGE_MAX_LATERAL_DISTANCE
            // (original L19473/L19477-L19478), so the polygon width depends on hit_dir.
            let normal = (-hit_dir.1 * INVERSE_ASPECT_RATIO, hit_dir.0 * ASPECT_RATIO);
            {
                let first_corner = (
                    my_pos.x + me_to_hit.0 - dir_norm.0 * strike_begins_before,
                    my_pos.y + me_to_hit.1 - dir_norm.1 * strike_begins_before,
                );
                let loop_d = Self::RIDER_CHARGE_LOOP_DISTANCE;
                let lat_d = Self::RIDER_CHARGE_MAX_LATERAL_DISTANCE;

                let p0 = crate::geo2d::pt(first_corner.0, first_corner.1);
                let p1 = crate::geo2d::pt(
                    first_corner.0 + dir_norm.0 * loop_d,
                    first_corner.1 + dir_norm.1 * loop_d,
                );
                let p2 = crate::geo2d::pt(
                    first_corner.0 + dir_norm.0 * loop_d + normal.0 * lat_d,
                    first_corner.1 + dir_norm.1 * loop_d + normal.1 * lat_d,
                );
                let p3 = crate::geo2d::pt(
                    first_corner.0 + normal.0 * lat_d,
                    first_corner.1 + normal.1 * lat_d,
                );

                let poly = geo::Polygon::new(
                    geo::LineString::from(vec![
                        (p0.x as f64, p0.y as f64),
                        (p1.x as f64, p1.y as f64),
                        (p2.x as f64, p2.y as f64),
                        (p3.x as f64, p3.y as f64),
                        (p0.x as f64, p0.y as f64),
                    ]),
                    vec![],
                );

                use geo::Contains;
                // IsAnyFriendInThisPolygon walks every same-camp
                // fighter, skipping self and using the explicit `!IsDead
                // && !IsUnconscious` predicate (not the broader
                // `IsAbleToFight`). The strike polygon's first corner
                // collapses onto `pt_me` when `me_to_hit_norm <=
                // RIDER_CHARGE_LOOP_DISTANCE`, so without the self
                // exclusion `geo::Contains` can trip on the rider itself.
                for f in nearby_fighters {
                    if !f.is_friendly || f.handle == 0 {
                        continue;
                    }
                    if f.handle == self.base.me {
                        continue;
                    }
                    if f.is_dead || f.is_unconscious {
                        continue;
                    }
                    if f.position.level != my_pos.level {
                        continue;
                    }
                    let fp = geo::Point::new(f.position.x as f64, f.position.y as f64);
                    if poly.contains(&fp) {
                        return None;
                    }
                }
            }
        }

        let destination = Position {
            x: goal_x,
            y: goal_y,
            sector: my_pos.sector,
            level: my_pos.level,
        };

        // Near enough to begin strike?
        let sq_hit_dist = me_to_hit.0 * me_to_hit.0 + me_to_hit.1 * me_to_hit.1;
        let begin_charge_anim = sq_hit_dist < Self::RIDER_CHARGE_SQR_LOOP_DISTANCE;

        Some((destination, begin_charge_anim))
    }

    /// Compute a retreat position for a rider after a charge pass.
    ///
    /// The rider tries to ride as far as possible in its current direction,
    /// testing variations (straight, slight left, slight right).
    pub(super) fn get_good_rider_reattack_goal(
        &self,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> Option<Position> {
        let my_pos = ctx.position;
        let my_dir = ctx.direction;
        let pt_me = crate::geo2d::pt(my_pos.x, my_pos.y);

        // Try distances from MAX down to 10, testing directions 0, +1, -1
        // at each distance.
        let mut distance = Self::RIDER_MAX_REATTACK_DISTANCE;
        while distance > 10.0 {
            for &rel_dir in &[0i16, 1, -1] {
                // `(direction + relative_direction) % 15` is a known bug
                // in the reference (should be `% 16`); `SetSector0to15`
                // then masks with `& 15`. Reproduce the C truncated-mod
                // (`%` in C follows truncation toward zero — so does
                // Rust's `%` on signed integers), then cast to u16 and
                // mask so negative results wrap via two's-complement
                // like a UBYTE cast.
                let raw = ((my_dir as i32) + (rel_dir as i32)) % 15;
                let dir = (raw as u16) & 15;
                let v = sector_to_vector_iso(dir, ASPECT_RATIO);
                let gx = my_pos.x + v.0 * distance;
                let gy = my_pos.y + v.1 * distance;

                // IsStraightMovementAutorized.
                let clear = match grid {
                    Some(g) => g.is_straight_movement_authorized(
                        pt_me.into(),
                        crate::geo2d::pt(gx, gy).into(),
                        my_pos.level,
                        &ctx.move_box,
                    ),
                    None => true,
                };
                if clear {
                    return Some(Position {
                        x: gx,
                        y: gy,
                        sector: my_pos.sector,
                        level: my_pos.level,
                    });
                }
            }
            distance -= 10.0;
        }

        None
    }

    /// Handle reattack after a rider has passed through enemies and returned.
    pub(super) fn rider_reattack(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        self.reinitialize_them_list(ctx, tick);

        if self.list_them.is_empty() {
            // No enemies visible — ride to last known position
            self.set_state(
                AiState::Attacking,
                Substate::AttackingRiderChargingApproachingBlindly,
            );
            self.base
                .go_to(self.base.seek_position, GotoFlags::RUN, ctx);
        } else {
            // Enemies visible — reconsider battle
            self.battle_decisions(global, ctx, tick, grid);
        }
    }

    // -----------------------------------------------------------------------
    // BeginSwordfight
    // -----------------------------------------------------------------------

    pub fn begin_swordfight(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        if self.base.primary_target == 0 {
            tracing::warn!(
                current_state = ?self.base.current_state,
                current_substate = ?self.base.current_substate,
                "Enemy AI: begin_swordfight called with primary_target=0 — aborting; this usually means the AI transitioned to AttackingSwordfight without passing through event_view_standard_procedure (or primary_target got cleared between detection and approach)",
            );
            return;
        }
        // begin_swordfight assumes the AI is already committed to a
        // fight (BeginSwordFight runs only from the Attacking state
        // machine). If the caller got here from Default/Wondering,
        // that's a stale `reconsider_enemy_approach` tick firing against
        // a soldier whose state changed underneath it — don't launch
        // combat, just bail.
        if !matches!(self.base.current_state, AiState::Attacking) {
            tracing::warn!(
                current_state = ?self.base.current_state,
                current_substate = ?self.base.current_substate,
                primary_target = self.base.primary_target,
                "Enemy AI: begin_swordfight called from non-Attacking state — aborting (stale timer tick?)",
            );
            return;
        }
        tracing::info!(
            target = self.base.primary_target,
            jump_line = ?tick.primary_target_jump_line,
            "Enemy AI: entering swordfight"
        );
        self.base.stop_all();

        // Civilians within reach flinch / scatter as soon as the soldier
        // draws his sword via the standard approach path (not just the
        // EVENT_ENTER_SWORDFIGHT entry).
        self.nearby_civilians_panic();

        self.left_combat_neighbour = 0;
        self.right_combat_neighbour = 0;

        // Release eye-tracking lock on swordfight entry so the soldier's
        // focus arrow / cone stops chasing the previous focus target.
        self.base.pending_unfocus = true;

        // If the target is moving and not yet swordfighting, freeze it
        // via Stop() so the swordfight starts from a stable position.
        // The engine drains `pending_stop_target` before launching
        // ENTER_SWORDFIGHT, matching the reference ordering.
        if let Some(target) = self.find_fighter(self.base.primary_target, tick)
            && !target.is_swordfighting
            && target.action_state.is_moving()
        {
            self.base.pending_stop_target = Some(self.base.primary_target);
        }

        // No SetDirection here. Direction is set by the engine-side
        // ENTER_SWORDFIGHT pipeline: the
        // `RHANIMATION_TRANSITION_RAISING_SWORD` order carries the
        // opponent as `pAntagonist`, and the soldier's per-tick
        // execute handler calls `SetDirection(opponent - me)` on
        // initialisation, then `Turn()` rotates the body each frame.
        // Mirrored at the order-launch sites in
        // `EngineInner::dispatch_enter_swordfight` and
        // `EngineInner::enter_swordfight_with_jump_line`.

        // Store the jump-line for the ENTER_SWORDFIGHT handler
        // (RHFIELD_JUMPLINE_DESTINATION).
        self.my_line_jump = tick.primary_target_jump_line;

        // Tell the engine to call enter_swordfight(me, target) so both
        // entities get added to each other's opponent lists and action
        // states transition to sword combat. The jump-line index is
        // passed alongside for table-swordfight positioning.
        self.base.pending_enter_swordfight =
            Some(EnterSwordfightRequest::Engage(self.base.primary_target));
        self.base.pending_enter_swordfight_jump_line = tick.primary_target_jump_line;

        // VIPs use a different remark variant.
        if self.is_vip {
            self.base.say(Remark::VipStartsCombat);
        } else {
            self.base.say(Remark::StartsCombat);
        }
        self.base.clear_emoticon();
        self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        self.base.launch_timer(20, ctx.frame);
    }

    // -----------------------------------------------------------------------
    // EndSwordfight
    // -----------------------------------------------------------------------

    pub fn end_swordfight(&mut self, ctx: &AiContext, _tick: &AiPerTickData) {
        // If the entity is still swordfighting, launch a QUIT_SWORDFIGHT
        // sequence element to clear the opponent list and transition
        // action state. We can't call the engine directly, so we set a
        // pending flag that the engine picks up after the AI tick.
        if !ctx.is_swordfighting {
            return;
        }
        self.base.pending_quit_swordfight = true;
    }
}
