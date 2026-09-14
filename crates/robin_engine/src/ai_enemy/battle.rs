//! Battle decisions and combat execution.
//!
//! Contains the combat decision tree (`battle_decisions`,
//! `make_battle_predecisions`, `execute_battle_decision`,
//! `get_battle_overview`), enemy approach (`attack_enemy`,
//! `reconsider_enemy_approach`), rider charges (`maybe_make_rider_attack`
//! and helpers), the sleeping-enemy approach helpers, and the
//! swordfight begin/end transitions.

use crate::ai::*;
use crate::fast_find_grid::FastFindGrid;
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::sim_rng::SimulationContext;

use super::map_vec_ext::AiMapVec;
use super::util::vec_to_sector;
use super::{
    EnemyAi, FighterSnapshot, PrimaryTargetFlags, ProfileRank, SeekFlags, ThinkEnv,
    UNDEFINED_DIRECTION, archer, combat,
};
use crate::coordinates::MapVec;

/// Keep the battle-side decision trace independently gated from the engine
/// context trace. This diagnostic is process-local and stderr-only, so its
/// disabled path cannot alter state, RNG consumption, or serialization.
fn archer_step_back_lifecycle_debug_matches(
    frame: u32,
    creation_order: Option<u32>,
    owner_handle: u32,
) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_ARCHER_STEP_BACK_LIFECYCLE",
            [
                "PARITY_DEBUG_ARCHER_STEP_BACK_FRAME",
                "PARITY_DEBUG_ARCHER_STEP_BACK_CREATION_ORDER",
                "PARITY_DEBUG_ARCHER_STEP_BACK_OWNER_HANDLE",
            ],
        )
    });
    gate.enabled() && gate.matches_required([Some(frame), creation_order, Some(owner_handle)])
}

/// Enemy-approach reconsideration uses raw saved-position map coordinates and stores
/// their Euclidean norm in an unsigned 16-bit value. This is deliberately different from the
/// game's general aspect-corrected distance helpers.
fn reconsider_approach_distance(a: Position, b: Position) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let truncated = (dx * dx + dy * dy).sqrt() as u16;
    f32::from(truncated)
}

fn enough_nearer_friends_to_observe(
    nearer_friends: u16,
    visible_enemies: usize,
    courage: u16,
) -> bool {
    let visible_enemies = visible_enemies as f32;
    // Preserve Original's floating comparison and operation grouping:
    //   friends >= enemies + enemies * (0.045f * courage)
    // Truncating the courage bonus first lets a soldier observe with too few
    // friends at every non-integral threshold.
    f32::from(nearer_friends)
        >= visible_enemies + visible_enemies * (0.045_f32 * f32::from(courage))
}

/// Derive the original game's nearby-alerting-soldier state from the friends already admitted
/// to the ally list. The admission walk has performed the authoritative 360-degree
/// detection query; querying the camp again here changes both call order and
/// the opaque-visibility cache.
fn has_nearby_alerting_soldier(
    owner: NpcHandle,
    admitted_friends: &[HumanHandle],
    candidates: impl IntoIterator<Item = (NpcHandle, Substate)>,
) -> bool {
    candidates.into_iter().any(|(handle, substate)| {
        handle != owner
            && admitted_friends.contains(&handle)
            && substate == Substate::SeekingRunningToOfficer
    })
}

/// The original game compares the actors' literal squared distance
/// 3D sprite positions, stretches world Y, includes Z, and then truncates the
/// single-precision result to an unsigned 32-bit value before comparing friend distances.
pub(crate) fn battle_owner_target_square_distance(
    owner: crate::coordinates::WorldPoint3D,
    target: crate::coordinates::WorldPoint3D,
) -> u32 {
    let dx = target.x - owner.x;
    let dy = (target.y - owner.y) * INVERSE_ASPECT_RATIO;
    let dz = target.z - owner.z;
    (dx * dx + dy * dy + dz * dz) as u32
}

pub(crate) fn battle_friend_is_nearer(
    friend: Position,
    target: Position,
    owner_target_square_distance: u32,
) -> bool {
    let dx = friend.x - target.x;
    let dy = friend.y - target.y;
    dx * dx + dy * dy < owner_target_square_distance as f32
}

/// Increment primary-target multiplicity: every nearby friend in the
/// broad swordfight family adds another `UNOCCUPIED_PREFERRED` penalty.
pub(crate) fn increment_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
) {
    let count = multiplicity.entry(target).or_insert(0);
    // The original game stores this counter in an unsigned 16-bit value.
    *count = u32::from((*count as u16).wrapping_add(1));
}

/// Preserve the shared counter for a target appended after battle planning's
/// reset pass. Original resets multiplicity only for the enemies already in
/// enemy list; a nearby friend's previously unseen target retains its live
/// global value when it is inserted later in the same decision.
pub(crate) fn seed_appended_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
    shared_multiplicity: &std::collections::BTreeMap<HumanHandle, u32>,
) {
    multiplicity
        .entry(target)
        .or_insert_with(|| shared_multiplicity.get(&target).copied().unwrap_or(0));
}

/// Preserve shot-target selection's use of the actors' shared multiplicity scratch:
/// clear all current enemies, then count only friends actively using a bow.
/// A failed shot proposal can immediately fall through to another battle
/// decision, so that later selector must observe this rebuilt state.
fn rebuild_battle_target_multiplicity_for_shot(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    enemies: &[HumanHandle],
    bow_targets: impl IntoIterator<Item = HumanHandle>,
) {
    multiplicity.clear();
    for &enemy in enemies {
        multiplicity.insert(enemy, 0);
    }
    for target in bow_targets {
        increment_battle_target_multiplicity(multiplicity, target);
    }
}

/// Return the live primary-target claim used by battle decisions' friend
/// scan. This is deliberately independent of both the friend's swordfight
/// opponent list and any earlier enemy-attack target recorded during the
/// same owner pass: later AI work can retarget the primary target while the
/// melee opponent remains unchanged.
fn battle_friend_primary_target(
    state: AiState,
    primary_target: Option<AiEntityHandle>,
) -> Option<HumanHandle> {
    (state == AiState::Attacking)
        .then_some(primary_target)
        .flatten()
        .map(AiEntityHandle::get)
}

impl EnemyAi {
    pub(crate) fn enter_battle_reserve(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        self.enter_battle_reserve_with_multiplicity(ctx, tick, None);
    }

    fn enter_battle_reserve_with_multiplicity(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        target_multiplicity: Option<&std::collections::BTreeMap<HumanHandle, u32>>,
    ) {
        let target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            ctx,
            tick,
            target_multiplicity,
        );
        self.base.primary_target = target;
        self.base.outbox.actor.set_focus(target);
        self.set_state_with_timer(AiState::Attacking, Substate::AttackingReserve, 50, ctx);
    }

    // -----------------------------------------------------------------------
    // Attack nearby sleeping enemies
    // -----------------------------------------------------------------------

    /// Release the actor borrow before duty and the following live fighter scan.
    fn kill_nearby_sleeping_enemies(&mut self, env: ThinkEnv<'_>) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::ScanSleepingEnemies {
                observer_camp: env.ctx.camp,
            },
            after: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------
    // Battle overview
    // -----------------------------------------------------------------------

    pub(crate) fn get_battle_overview(
        &mut self,
        flags: u16,
        _env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: DutyTail::BattleOverview { flags },
            after: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------
    // Battle predecisions — offensive or defensive?
    // -----------------------------------------------------------------------

    pub(crate) fn make_battle_predecisions(&mut self, env: ThinkEnv<'_>) -> Decision {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        // Archers with no ammo or already swordfighting → defensive.
        if self.is_archer() && (ctx.remaining_arrows == 0 || ctx.is_swordfighting) {
            return Decision::PredecisionDefensive;
        }

        // Already fleeing → defensive
        if self.base.current_state == AiState::Fleeing {
            return Decision::PredecisionDefensive;
        }

        // --------- US ---------
        // The original game walks the persistent ally list at this exact point.
        // Deriving the aggregate while constructing generic tick snapshots
        // both made stale list assumptions and used to trigger eager LOS.
        let mut us_points = 0_u16;
        let mut there_is_an_officer = false;
        for &friend_handle in &self.base.list_us {
            let friend = ctx.entity_view(friend_handle).unwrap_or_else(|| {
                panic!(
                    "battle predecision friendly-list member {} is absent from the AI entity view",
                    friend_handle
                )
            });
            if friend.is_pc {
                us_points = us_points.wrapping_add(100);
                continue;
            }
            let (pride, rank) = if friend_handle == self.base.me {
                (self.soldier_profile_pride, self.get_rank())
            } else {
                let soldier = tick
                    .camp_soldiers
                    .iter()
                    .find(|soldier| soldier.handle == friend_handle)
                    .unwrap_or_else(|| {
                        panic!(
                            "battle predecision soldier {} is absent from camp_soldiers",
                            friend_handle
                        )
                    });
                (soldier.pride, soldier.rank)
            };
            us_points = us_points.wrapping_add(100_u16.wrapping_add(pride));
            there_is_an_officer |= friend_handle != self.base.me && rank == ProfileRank::Officer;
        }

        self.battle_predecision_from_points(
            sim,
            us_points,
            self.list_them.len() as u16,
            there_is_an_officer,
            ctx.self_life_points,
            ctx.self_max_life_points,
        )
    }

    pub(crate) fn battle_predecision_from_points(
        &self,
        sim: &SimulationContext,
        us_points: u16,
        enemies: u16,
        there_is_an_officer: bool,
        life_points: i16,
        max_life_points: i16,
    ) -> Decision {
        let them_points = enemies.wrapping_mul(100).wrapping_add(1);
        let relation = (u32::from(us_points) * 100 / u32::from(them_points)) as u16;
        let mut odds = if relation >= 100 {
            let raw =
                (50 + 50 * (i32::from(relation) - 100)
                    / parameters_ai::AI_BEST_BATTLE_RELATION_MINUS_100) as i16;
            raw.min(100)
        } else {
            let raw = (50 * (i32::from(relation) - parameters_ai::AI_WORST_BATTLE_RELATION)
                / parameters_ai::AI_100_MINUS_WORST_BATTLE_RELATION) as i16;
            raw.max(0)
        };
        if life_points < max_life_points {
            odds = (i32::from(odds) * i32::from(life_points) / i32::from(max_life_points)) as i16;
        }
        if self.get_rank() == ProfileRank::Soldier && there_is_an_officer {
            odds = (i32::from(odds) * combat::OFFICER_ODDS_BONUS) as i16;
        }
        let courage = self.get_courage();
        if i32::from(odds) < (50 - i32::from(courage) / 2)
            && crate::sim_rng::u16(sim, crate::sim_rng::RngSite::BattleCourage, 0..100) > courage
        {
            Decision::PredecisionDefensive
        } else {
            Decision::PredecisionOffensive
        }
    }

    // -----------------------------------------------------------------------
    // Battle decisions — the heart of tactical AI
    // -----------------------------------------------------------------------

    pub(crate) fn battle_decisions(
        &mut self,
        _env: ThinkEnv<'_>,
        _global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let mut call = crate::ai::DutyCall::new(crate::ai::DutyFlags::empty(), false);
        call.tail = crate::ai::DutyTail::BattleDecisions;
        Err(call)
    }

    pub(crate) fn finish_battle_decisions(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
        old_substate: Substate,
        inputs: BattleDecisionInputs,
        mut decision_target_multiplicity: std::collections::BTreeMap<HumanHandle, u32>,
        unconscious_enemies_from_them: Vec<HumanHandle>,
    ) -> crate::ai::AiFlow<()> {
        let ctx = env.ctx;
        let BattleDecisionInputs {
            friends_lower_company,
            soldiers_lower_pride,
            simple_soldiers_near,
            min_square_enemy_distance,
            num_enemies_i_can_see,
            friends_nearer_to_enemy,
        } = inputs;
        if num_enemies_i_can_see == 0 {
            self.battle_no_visible_enemies(env, global, unconscious_enemies_from_them)?;
            return Ok(());
        }

        let (decision, cover_shield_bearer) = self.choose_battle_decision(
            env,
            global,
            BattleDecisionInputs {
                friends_lower_company,
                soldiers_lower_pride,
                simple_soldiers_near,
                min_square_enemy_distance,
                num_enemies_i_can_see,
                friends_nearer_to_enemy,
            },
            &decision_target_multiplicity,
        );

        tracing::trace!(
            me = self.base.me,
            ?decision,
            primary_target = ?self.base.primary_target,
            num_enemies_i_can_see,
            friends_nearer_to_enemy,
            soldiers_lower_pride = soldiers_lower_pride,
            friends_lower_company = friends_lower_company,
            "battle_decisions: chose decision"
        );
        if crate::ai_enemy::battle_decision_debug_enabled() {
            crate::ai_enemy::parity_trace::BattleDecision {
                frame: &(ctx.frame),
                me: &(self.base.me),
                decision: &(decision),
                old_substate: &(old_substate),
                primary: &(self.base.primary_target),
                seen: &(num_enemies_i_can_see),
                friends_nearer: &(friends_nearer_to_enemy),
            }
            .emit();
        }
        // Carry out decision (with possible fallback loop). The Observe
        // arm's avenger-on-roof fallback returns from the whole routine
        // before the log line is registered; every other path logs.
        if let Some(decision) = self.execute_battle_decision(
            env,
            decision,
            old_substate,
            cover_shield_bearer,
            &mut decision_target_multiplicity,
            global,
        )? {
            self.base
                .register_log_line(LogLineType::BattleDecision, decision as u16);
        }
        Ok(())
    }

    /// Battle planning with no personally visible enemies.
    fn battle_no_visible_enemies(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
        unconscious_enemies_from_them: Vec<HumanHandle>,
    ) -> crate::ai::AiFlow<()> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        // No visible enemies. Ordering:
        //   combat_trainer → my_shooting_point → archer-leaning-out
        //   → friends-see-enemies (seek) → missed-PC → unconscious
        //   → kill_nearby_sleeping. archer-leaning-out MUST come
        //   before the seek-friends-enemies arm — an archer parked
        //   on a bend point with friend-seen enemies should hold
        //   the firing position, not run away to seek.
        if self.combat_trainer {
            self.return_to_duty_default(env)?;
        } else if self.my_shooting_point.is_some() {
            // Archer has a shooting point — equip bow based on
            // elevation relative to last-seen enemy.
            let my_elevation: u16 = ctx.elevation as u16;
            if my_elevation >= self.enemy_had_this_elevation + 50 {
                // Target is below — aim down
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::EquipBowDown);
                self.set_state(
                    AiState::Attacking,
                    Substate::AttackingArcherWaitOnArcheryPathBending,
                );
            } else {
                // Target is at same level or above
                self.base
                    .outbox
                    .actor
                    .launch_commands
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
            self.set_state_with_timer(
                AiState::Attacking,
                Substate::AttackingArcherWaitOnBendPoint,
                500,
                ctx,
            );
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
                env,
                self.base.seek_position,
                parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST,
                UNDEFINED_DIRECTION,
                global,
            )?;
        } else if self.pc_missed
            && self.missed_pc.is_some()
            && tick.missed_pc_is_pc
            && self.answer_question(Question::ShallIFollowLostEnemy, ctx)
        {
            // Lost enemy — re-forecast and seek with direction hint.
            self.base.say(Remark::HuntsEnemy);
            // Re-predict missed PC's destination before seeking. A
            // synchronous queued Think can assign `missed_pc` after its
            // per-tick snapshot was built, so use the handle-keyed
            // detectable/primary forecast already prepared for that
            // target before the snapshot's dedicated convenience slot.
            // The original game forecasts the AI destination unconditionally;
            // retaining an old seek position is not a valid fallback.
            self.refresh_missed_pc_forecast(sim, tick);
            self.seek_area(
                env,
                self.base.seek_position,
                parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                self.pc_gone_away_in_this_direction,
                global,
            )?;
        } else if !unconscious_enemies_from_them.is_empty() && !self.is_merry_man_forest(ctx) {
            // Enemies removed from the persistent Them list above are
            // unconscious and not carried — put them back, select one,
            // and walk up to finish them off.
            debug_assert!(self.list_them.is_empty());
            return Err(crate::ai::DutyCall {
                flags: DutyFlags::empty(),
                think_result: false,
                tail: crate::ai::DutyTail::ApproachSleepingEnemies {
                    targets: unconscious_enemies_from_them,
                },
                after: Vec::new(),
            });
        } else {
            // Final "there is literally nothing going on" fallback —
            // look for sleeping enemies anywhere within the 360°
            // detection radius and walk over to one.
            self.kill_nearby_sleeping_enemies(env)?;
        }
        Ok(())
    }

    /// Choose the battle decision: forced-decision whitelist, then the
    /// offensive/defensive predecision split.
    fn choose_battle_decision(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
        inputs: BattleDecisionInputs,
        decision_target_multiplicity: &std::collections::BTreeMap<HumanHandle, u32>,
    ) -> (Decision, HumanHandle) {
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
            // The original game never consumes this value. Although
            // Forcing the next battle decision also stores a reset flag,
            // Battle planning does not read that flag or clear the forced
            // value. A non-`None` decision therefore remains forced on every
            // later pass until a script replaces it.
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
                // best-effort recovery.
                // Simulate "no forced decision" by jumping into the
                // else block via a goto-style early flag.
                let predecision = self.make_battle_predecisions(env);
                decision = if self.combat_trainer || predecision == Decision::PredecisionDefensive {
                    Decision::Cassos
                } else {
                    Decision::Fight
                };
            }
        } else {
            // (1) Predecision: Offensive or defensive?
            let predecision = self.make_battle_predecisions(env);

            if self.combat_trainer {
                decision = Decision::Observe;
            } else if predecision == Decision::PredecisionOffensive {
                decision = self.battle_offensive_decision(
                    env,
                    global,
                    inputs,
                    decision_target_multiplicity,
                    &mut cover_shield_bearer,
                );
            } else {
                decision = self.battle_defensive_decision(env);
            }
        }
        (decision, cover_shield_bearer)
    }

    /// Offensive half of the battle decision tree (archer, tower guard,
    /// officer alert, reserve, pride, observe, fight).
    fn battle_offensive_decision(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
        inputs: BattleDecisionInputs,
        decision_target_multiplicity: &std::collections::BTreeMap<HumanHandle, u32>,
        cover_shield_bearer: &mut HumanHandle,
    ) -> Decision {
        let ThinkEnv { ctx, tick, .. } = env;
        let BattleDecisionInputs {
            friends_lower_company,
            soldiers_lower_pride,
            simple_soldiers_near,
            min_square_enemy_distance,
            num_enemies_i_can_see,
            friends_nearer_to_enemy,
        } = inputs;
        let decision;
        // Use the aggregates computed by this decision's camp scan.
        let friends_with_lower_company = friends_lower_company;
        let soldiers_with_lower_pride = soldiers_lower_pride;

        ////////// offensive decisions //////////////

        if self.is_archer() && self.base.blood_alcohol == 0 {
            if crate::ai_enemy::battle_decision_debug_enabled() {
                crate::ai_enemy::parity_trace::ArcherDecision {
                    frame: &(ctx.frame),
                    me: &(self.base.me),
                    tower: &(self.tower_guard),
                    sbb: &(self.shield_bearer_before_me),
                    shooting_point: &(self.my_shooting_point),
                    too_near: &(self.base.primary_target.is_some()
                        && self.archer_is_too_near_to_enemy(
                            &ctx.position,
                            self.base.primary_target,
                            ctx,
                            tick,
                        )),
                    pos: &(ctx.position),
                    primary: &(self.base.primary_target),
                    primary_pos: &(self
                        .find_fighter(self.base.primary_target, tick)
                        .map(|f| f.position)),
                }
                .emit();
            }
            // Archer offensive.
            if self.tower_guard {
                if !self.base.friends_are_alerted {
                    decision = Decision::TowerGuardAlert;
                } else {
                    decision = Decision::Shoot;
                }
            } else if self.base.primary_target.is_some()
                && self.archer_is_too_near_to_enemy(
                    &ctx.position,
                    self.base.primary_target,
                    ctx,
                    tick,
                )
            {
                // Step back and decide again.
                decision = Decision::ArcherStepBack;
            } else if self.shield_bearer_before_me.is_some() && self.base.blood_alcohol == 0 {
                // Already paired with a shield bearer — check if
                // we're still in cover or need to reposition.
                if let Some(cover_pos) =
                    self.shield_bearer_cover_position(self.shield_bearer_before_me, tick)
                {
                    let diff = ctx.position.map_point() - cover_pos.map_point();
                    if diff.max_norm() < archer::COVER_POINT_TOLERANCE as f32 {
                        // Still in cover — shoot
                        decision = Decision::Shoot;
                    } else {
                        // Need to reposition behind shield bearer
                        *cover_shield_bearer = self
                            .shield_bearer_before_me
                            .expect("active archer cover has no shield bearer")
                            .get();
                        decision = Decision::CoverBehindShieldBearer;
                    }
                } else {
                    // Shield bearer lost or unreachable
                    self.update_shield_bearer_before_me(None);
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
                    *cover_shield_bearer = sb;
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
            } else if min_square_enemy_distance < combat::MIN_SQUARE_RESERVE_DISTANCE as u32 {
                decision = Decision::Fight;
            } else {
                decision = Decision::TowerGuardObserve;
            }
        } else if self.get_rank() == ProfileRank::Officer
            && simple_soldiers_near
            && !self.base.friends_are_alerted
            && self.base.blood_alcohol == 0
        {
            // Officer alerts soldiers (only if simple soldiers are nearby).
            decision = Decision::AlertSoldiers;
        } else if friends_with_lower_company >= self.list_them.len() as u16
            && min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE as u32
        {
            // Enough friends closer → hold back.
            decision = Decision::Reserve;
        } else if self.company_number == 100
            && min_square_enemy_distance > combat::MIN_SQUARE_RESERVE_DISTANCE as u32
        {
            // Company 100 → last reserve.
            decision = Decision::LastReserve;
        } else if soldiers_with_lower_pride
            && self.is_too_proud_to_attack(ctx, tick, Some(decision_target_multiplicity))
        {
            // Too proud to fight alongside commoners.
            decision = Decision::TooProudToAttack;
        } else if ctx.is_hostile_to_player()
            && !soldiers_with_lower_pride
            && enough_nearer_friends_to_observe(
                friends_nearer_to_enemy,
                num_enemies_i_can_see,
                self.get_courage(),
            )
        {
            // Lacklandist observe — enough friends are already
            // fighting closer to the enemy, stand back and watch.
            // Camp-gated: only Lacklandists take this branch;
            // royalists fall through to Fight.
            // `num_enemies_i_can_see` is a persistent count of
            // tracked enemies, not a per-tick "detected this
            // frame" count; otherwise EVENT_TIMER-driven calls would see
            // `0 >= 0 + 0 = true` and wrongly observe instead of
            // charging.
            decision = Decision::Observe;
        } else {
            // Charge! (Earlier port versions injected a
            // `refresh_arrow_protection` early-return here, but
            // the offensive-decision chain does not call
            // arrow-protection refresh — that sweep lives in
            // the every-16-frame update and a few explicit call sites.)
            decision = Decision::Fight;
        }
        decision
    }

    /// Defensive half of the battle decision tree: arrows, then help/alert
    /// by rank, otherwise Cassos.
    fn battle_defensive_decision(&self, env: ThinkEnv<'_>) -> Decision {
        let ThinkEnv { ctx, tick, .. } = env;
        let decision;
        // `only_enemy_soldiers` is initialized true, cleared if
        // any PC is in list_them. Used to gate LookForHelp /
        // RunAndAlertSoldiers — you don't call for help if your
        // opponents are all enemy soldiers (friendly fire / brawl
        // semantics).
        let only_enemy_soldiers = !self.list_them.iter().any(|&h| {
            self.find_fighter_logged(h, tick, "enemy-list PC scan")
                .is_some_and(|f| f.is_pc)
        });

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
        decision
    }

    fn refresh_missed_pc_forecast(&mut self, sim: &SimulationContext, tick: &AiPerTickData) {
        let missed_pc = self
            .missed_pc
            .expect("lost-PC forecast refresh requires a missed PC")
            .get();
        let detectable = tick
            .enemy_detectable_forecasts
            .iter()
            .find_map(|(handle, forecast)| (*handle == missed_pc).then_some(forecast));
        let primary = (tick.primary_target_snapshot_handle == Some(AiEntityHandle::new(missed_pc)))
            .then_some(tick.primary_target_forecast.as_ref())
            .flatten();
        let dedicated = (tick.missed_pc_forecast_handle == Some(AiEntityHandle::new(missed_pc)))
            .then_some(tick.missed_pc_forecast.as_ref())
            .flatten();
        let prepared = detectable.or(primary).or(dedicated).unwrap_or_else(|| {
            panic!(
                "NPC {} lost-PC overview target {} has no prepared destination forecast",
                self.base.me, missed_pc
            )
        });
        let forecast =
            prepared.resolve_retaining_direction(sim, self.pc_gone_away_in_this_direction);
        self.base.seek_position = forecast.position;
        self.pc_gone_away_in_this_direction = forecast.direction;
    }

    /// Execute a battle decision, with fallback to alternative decisions if needed.
    /// `cover_shield_bearer` is the handle of the shield bearer chosen during the
    /// decision phase for `CoverBehindShieldBearer`; 0 for all other decisions.
    ///
    /// Returns `false` when the caller must not register the current decision:
    /// the Observe arm's avenger-on-roof fallback skips it, while deferred
    /// Looking for help registers exactly one final decision after officer alerting's
    /// route result is known. Every other path returns `true`.
    pub(crate) fn execute_battle_decision(
        &mut self,
        env: ThinkEnv<'_>,
        mut decision: Decision,
        old_substate: Substate,
        cover_shield_bearer: HumanHandle,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<Option<Decision>> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        // Allow up to 5 fallback decision changes to prevent infinite loops
        for _ in 0..5 {
            match decision {
                Decision::Fight => match self.execute_fight_decision(target_multiplicity, env)? {
                    std::ops::ControlFlow::Continue(next) => {
                        decision = next;
                        continue;
                    }
                    std::ops::ControlFlow::Break(result) => return Ok(result.then_some(decision)),
                },

                Decision::Reserve => {
                    self.enter_battle_reserve_with_multiplicity(
                        ctx,
                        tick,
                        Some(target_multiplicity),
                    );
                }

                Decision::LastReserve => {
                    match self.execute_last_reserve_decision(target_multiplicity, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::Observe => {
                    match self.execute_observe_decision(target_multiplicity, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::Shoot => {
                    if ctx.remaining_arrows == 0 {
                        decision = Decision::RunForNewArrows;
                        continue;
                    }
                    return Err(crate::ai::DutyCall {
                        flags: crate::ai::DutyFlags::empty(),
                        think_result: false,
                        tail: crate::ai::DutyTail::SelectShotTarget {
                            old_substate,
                            cover_shield_bearer,
                        },
                        after: Vec::new(),
                    });
                }

                Decision::Cassos => {
                    // In Merry Man Forest, try to flee via
                    // forest retreat first. Otherwise: random
                    // Cassos/Panic remark, pick a primary target, then
                    // Panic(target_pos, AI_STANDARD_PANIC_RUNS) — note
                    // the threat point is the target's *current*
                    // position, NOT seek_position.
                    if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global)
                    {
                        // The original game randomly chooses between the two panic variants.
                        if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
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
                        // Speech runs before the original game reselects the target and can
                        // synchronously invalidate the battlefield membership
                        // that selected CASSOS. Original then uses the existing
                        // undirected `Panic(runs)` overload when reselection is
                        // empty; handle 0 selects that overload here.
                        self.begin_cassos_panic(target.map_or(0, AiEntityHandle::get), ctx);
                    }
                }

                Decision::LookForHelp => match self.execute_look_for_help_decision(env)? {
                    std::ops::ControlFlow::Continue(next) => {
                        decision = next;
                        continue;
                    }
                    std::ops::ControlFlow::Break(result) => return Ok(result.then_some(decision)),
                },

                Decision::AlertSoldiers => match self.execute_alert_soldiers_decision(env)? {
                    std::ops::ControlFlow::Continue(next) => {
                        decision = next;
                        continue;
                    }
                    std::ops::ControlFlow::Break(result) => return Ok(result.then_some(decision)),
                },

                Decision::RunAndAlertSoldiers => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    let center = ctx
                        .expect_entity_view(
                            target.expect("RunAndAlertSoldiers requires a primary target"),
                            "run-and-alert-soldiers primary target",
                        )
                        .position;
                    return Err(crate::ai::DutyCall {
                        flags: crate::ai::DutyFlags::empty(),
                        think_result: false,
                        tail: crate::ai::DutyTail::RunAndAlertSoldiers { center },
                        after: Vec::new(),
                    });
                }

                Decision::TowerGuardAlert => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    // Primary-target replacement can legally return the null handle
                    // after the overview selected this decision. The recorder
                    // mirrors Original's existing failed-decision path and
                    // retries as reserve; do not turn that sentinel into a
                    // required entity-view lookup.
                    let Some(target) = target else {
                        tracing::warn!(
                            me = self.base.me,
                            "tower-guard alert lost its primary target; reserving instead"
                        );
                        decision = Decision::Reserve;
                        continue;
                    };
                    self.base.friends_are_alerted = true;
                    self.base.seek_position = ctx
                        .expect_entity_view(target, "tower-guard alert primary target")
                        .position;
                    self.set_state(AiState::Attacking, Substate::AttackingTowerGuardAlert);
                    self.base.point_to(self.base.seek_position, ctx);
                }

                Decision::TowerGuardObserve => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    let Some(target) = target else {
                        tracing::warn!(
                            me = self.base.me,
                            "tower-guard observation lost its primary target; reserving instead"
                        );
                        decision = Decision::Reserve;
                        continue;
                    };
                    self.base.friends_are_alerted = true;
                    self.base.seek_position = ctx
                        .expect_entity_view(target, "tower-guard observe primary target")
                        .position;
                    self.set_state(AiState::Attacking, Substate::AttackingTowerGuardObserve);
                    self.base.face_entity(target, ctx);
                    self.base.launch_timer(100, ctx.frame);
                }

                Decision::RunForNewArrows => {
                    match self.execute_run_for_new_arrows_decision(global, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::TooProudToAttack => {
                    match self.execute_too_proud_to_attack_decision(old_substate, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::ArcherStepBack => {
                    match self.execute_archer_step_back_decision(old_substate, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::ArcherObserve => {
                    match self.execute_archer_observe_decision(target_multiplicity, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::Menace => {
                    // Menace a PC in coma.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    let _target = target.expect("Menace decision requires a primary target");
                    self.set_state_with_timer(
                        AiState::Menacing,
                        Substate::MenacingPcInComa,
                        parameters_ai::AI_MENACING_PATIENCE as u32,
                        ctx,
                    );
                }

                Decision::CoverBehindShieldBearer => {
                    match self.execute_cover_behind_shield_bearer_decision(cover_shield_bearer, env)
                    {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
                    }
                }

                Decision::RunToArcheryPoint => {
                    match self.execute_run_to_archery_point_decision(global, env) {
                        std::ops::ControlFlow::Continue(next) => {
                            decision = next;
                            continue;
                        }
                        std::ops::ControlFlow::Break(result) => {
                            return Ok(result.then_some(decision));
                        }
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
        Ok(Some(decision))
    }

    /// Resume the `DECISION_FIGHT` tail after its nested
    /// reconsidered enemy-approach route has settled.
    pub(crate) fn resume_battle_fight_after_reconsider(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        if !self.base.couldnt_reachpoint {
            self.base
                .register_log_line(LogLineType::BattleDecision, Decision::Fight as u16);
            return Ok(());
        }

        // Enemy battle decisions clear the failed fight
        // approach and loops directly into DECISION_OBSERVE. Rebuild the
        // local multiplicities from the live scratch counters retained by
        // this owner boundary; Observe's target selection reads them.
        //
        // This continuation is still executing inside the battle-planning
        // caller's original-game decision tick. Observation's nearby movement calls through the
        // ordinary controller helper, which derives completion ownership
        // from Rust's temporarily-unwound recursion depth. Preserve the
        // authoritative ownership across that call so a second route failure
        // is surfaced as a could-not-reach-point event at enclosing decision-tick completion
        // through the ordinary stop path.
        self.base.couldnt_reachpoint = false;
        let mut target_multiplicity = self
            .list_them
            .iter()
            .copied()
            .map(|target| {
                (
                    target,
                    global
                        .primary_target_multiplicity_scratch
                        .get(&target)
                        .copied()
                        .unwrap_or(0),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let completed_inline = self.execute_battle_decision(
            ThinkEnv { grid: None, ..env },
            Decision::Observe,
            self.base.current_substate,
            0,
            &mut target_multiplicity,
            global,
        )?;
        if let Some(decision) = completed_inline {
            self.base
                .register_log_line(LogLineType::BattleDecision, decision as u16);
        }
        Ok(())
    }

    /// Execute the original game's two panic variants after primary-target
    /// selection. A non-null human-actor reference is read through
    /// `Point(target)` at this call site; retaining an older seek point is not
    /// a valid substitute if the selected actor cannot be resolved. A null
    /// target deliberately calls the undirected `Panic(runs)` overload.
    fn begin_cassos_panic(&mut self, target: HumanHandle, ctx: &AiContext) {
        let runs = parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        if target == 0 {
            tracing::warn!(
                me = self.base.me,
                "Cassos decision lost its primary target; panicking without a direction"
            );
            let was_already_fleeing = matches!(
                self.base.current_substate,
                Substate::FleeingPanic | Substate::FleeingRunToDoor
            );
            self.base.directed_panic = false;
            if !was_already_fleeing {
                self.set_state(AiState::Fleeing, Substate::FleeingPanic);
            }
            self.base.outbox.actor.begin_panic = Some(PanicRequest {
                center: None,
                runs,
                alert: AlertLevel::Red,
                is_new_panic: !was_already_fleeing,
            });
            return;
        }

        let threat = ctx
            .expect_entity_view(target, "Cassos selected primary target")
            .position;
        self.panic_from_position(threat, runs);
    }

    /// Resume the statement immediately following officer alerting's
    /// synchronous approach in `DECISION_LOOK_4_HELP`.
    ///
    /// Rust constructs cross-sector routes after releasing the AI borrow, so
    /// this tail must run at the owner boundary. In Original, a failed route
    /// is consumed by officer alerting itself and changes the decision to
    /// `CASSOS`; it is not delivered as `EVENT_COULDNT_REACHPOINT`.
    pub(crate) fn finish_battle_look_for_help(
        &mut self,
        env: ThinkEnv<'_>,
        accepted: bool,
        global: &mut AiGlobalState,
    ) {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        if accepted {
            if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
                self.base.say(Remark::Cassos);
            } else {
                self.base.say(Remark::Panic);
            }
            self.base
                .register_log_line(LogLineType::BattleDecision, Decision::LookForHelp as u16);
            return;
        }

        // Officer alerting clears the latch before returning false. The enclosing
        // decision loop then executes the ordinary CASSOS arm.
        if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global) {
            if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
                self.base.say(Remark::Cassos);
            } else {
                self.base.say(Remark::Panic);
            }
            let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
            self.base.primary_target = target;
            self.begin_cassos_panic(target.map_or(0, AiEntityHandle::get), ctx);
        }
        self.base
            .register_log_line(LogLineType::BattleDecision, Decision::Cassos as u16);
    }

    // -----------------------------------------------------------------------
    // Engage an enemy
    // -----------------------------------------------------------------------

    pub(crate) fn attack_enemy(
        &mut self,
        enemy: HumanHandle,
        _env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<()> {
        let mut call = crate::ai::DutyCall::new(crate::ai::DutyFlags::empty(), false);
        call.tail = crate::ai::DutyTail::AttackEnemy { target: enemy };
        Err(call)
    }

    // -----------------------------------------------------------------------
    // Reconsider the enemy approach for melee
    // Simplified enemy-approach reconsideration.
    // -----------------------------------------------------------------------

    /// Decide how to approach the primary target: run when far, walk
    /// when close, fight when in melee range.
    ///
    /// Distance is sampled here from the target-specific live position.
    /// `seek_position` must already be set to the target's position
    /// before calling.
    ///
    /// Rider charge is handled by `maybe_make_rider_attack` (called
    /// from `attack_enemy`). Line-jump data is precomputed by the engine
    /// in `AiPerTickData::primary_target_jump_line`.
    pub(crate) fn reconsider_enemy_approach(
        &mut self,
        reachpoint: bool,
        _env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<()> {
        let mut call = crate::ai::DutyCall::new(crate::ai::DutyFlags::empty(), false);
        call.tail = crate::ai::DutyTail::ReconsiderEnemyApproach { reachpoint };
        Err(call)
    }

    /// Resume the original game's observation decision immediately after its
    /// first synchronous approach. A successful/no-roof path registers the
    /// decision once; the avenger-on-roof branch returns before that log.
    pub(crate) fn resume_battle_observe_after_go_near(
        &mut self,
        target: HumanHandle,
        target_position: Position,
        avenger_wait_position: Option<Position>,
        ctx: &AiContext,
    ) {
        assert_ne!(target, 0, "DECISION_OBSERVE continuation requires a target");
        assert_eq!(
            self.base.primary_target,
            Some(AiEntityHandle::new(target)),
            "DECISION_OBSERVE continuation target ownership changed"
        );

        self.base.set_emoticon(EmoticonType::XMark);
        self.set_state_with_timer(
            AiState::Attacking,
            Substate::AttackingApproachToObserve,
            50,
            ctx,
        );

        if self.base.couldnt_reachpoint
            && let Some(wait_pos) = avenger_wait_position
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
            self.base.seek_position = target_position;
            return;
        }

        self.base
            .register_log_line(LogLineType::BattleDecision, Decision::Observe as u16);
    }

    /// Compute the approach point on `line_idx` closest to the victim.
    /// Returns the point on the aggressor's jump-line B-end mirrored
    /// from the victim's nearest-point projection on the paired line.
    pub(crate) fn compute_jump_line_target(
        &self,
        grid: &FastFindGrid,
        line_idx: u32,
        victim_pos: crate::ai::Position,
    ) -> Option<crate::ai::Position> {
        let aggressor_line = grid.level.jump_lines.get(line_idx as usize)?;
        let victim_line_idx = aggressor_line.associated_line_index?;
        let victim_line = grid.level.jump_lines.get(victim_line_idx as usize)?;
        let t_victim = victim_line.compute_nearest_point_param(crate::coordinates::MapPoint::new(
            victim_pos.x,
            victim_pos.y,
        ));
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

    /// Compute the charge destination for a rider attacking a specific enemy.
    ///
    /// The rider charges past the enemy at a lateral offset, so the hit zone
    /// polygon sweeps across the enemy. Returns `(destination, begin_charge_anim)`.

    /// Compute a retreat position for a rider after a charge pass.
    ///
    /// The rider tries to ride as far as possible in its current direction,
    /// testing variations (straight, slight left, slight right).
    pub(super) fn get_good_rider_reattack_goal(&self, env: ThinkEnv<'_>) -> Option<Position> {
        let ThinkEnv { ctx, grid, .. } = env;
        let my_pos = ctx.position;
        let my_dir = ctx.direction;
        let pt_me = crate::coordinates::MapPoint::new(my_pos.x, my_pos.y);

        // Try distances from MAX down to 10, testing directions 0, +1, -1
        // at each distance.
        let mut distance = Self::RIDER_MAX_REATTACK_DISTANCE;
        while distance > 10.0 {
            for &rel_dir in &[0i16, 1, -1] {
                // `(direction + relative_direction) % 15` is a known bug
                // in the original game (should be `% 16`); direction-sector assignment
                // then masks with `& 15`. Reproduce the C truncated-mod
                // (`%` in C follows truncation toward zero — so does
                // Rust's `%` on signed integers), then cast to u16 and
                // mask so negative results wrap via two's-complement
                // like a UBYTE cast.
                let raw = ((my_dir as i32) + (rel_dir as i32)) % 15;
                let dir = (raw as u16) & 15;
                let v = MapVec::from_sector_iso(dir);
                let gx = my_pos.x + v.x * distance;
                let gy = my_pos.y + v.y * distance;

                // straight-movement authorization.
                let clear = match grid {
                    Some(g) => g.is_straight_movement_authorized(
                        pt_me,
                        crate::coordinates::MapPoint::new(gx, gy),
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
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let ctx = env.ctx;
        self.reinitialize_them_list(ctx);

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
            self.battle_decisions(env, global)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Enter swordfight
    // -----------------------------------------------------------------------

    pub fn begin_swordfight(&mut self, ctx: &AiContext) {
        if self.base.primary_target.is_none() {
            tracing::warn!(
                current_state = ?self.base.current_state,
                current_substate = ?self.base.current_substate,
                "Enemy AI: begin_swordfight called with primary_target=0 — aborting; this usually means the AI transitioned to AttackingSwordfight without passing through event_view_standard_procedure (or primary_target got cleared between detection and approach)",
            );
            return;
        }
        tracing::info!(
            target = ?self.base.primary_target,
            jump_line = ?self.my_line_jump,
            "Enemy AI: entering swordfight"
        );
        self.base.stop_all();

        // Civilians within reach flinch / scatter as soon as the soldier
        // draws his sword via the standard approach path (not just the
        // EVENT_ENTER_SWORDFIGHT entry).
        self.nearby_civilians_panic();

        // The original game enters swordfight by updating both reciprocal
        // setters, not by dropping only this soldier's local pointers
        // for this branch.
        self.clear_combat_neighbours();

        // Release eye-tracking lock on swordfight entry so the soldier's
        // focus arrow / cone stops chasing the previous focus target.
        self.base.outbox.actor.set_unfocus();

        // Ask the engine to apply swordfight entry's conditional target stop
        // at the outbox-drain boundary. The original game reads the target's live
        // action state here, after every earlier-created entity has already
        // run its update. `tick.fighters` is the pre-entity snapshot and
        // can still say Waiting when the target completed its start-running
        // transition earlier in this same frame. The engine therefore owns
        // both live gates (not swordfighting and Moving/MovingFast).
        self.base.outbox.actor.stop_target = Some(
            self.base
                .primary_target
                .expect("swordfight entry target presence was checked"),
        );

        // No direction change here. Direction is set by the engine-side
        // ENTER_SWORDFIGHT pipeline: the
        // raising-sword transition order carries the
        // opponent as the antagonist, and the soldier's per-tick
        // execution points the actor toward the opponent on
        // initialisation, then `Turn()` rotates the body each frame.
        // Mirrored at the order-launch sites in
        // `EngineInner::dispatch_enter_swordfight` and
        // `EngineInner::enter_swordfight_with_jump_line`.

        // Tell the engine to call enter_swordfight(me, target) so both
        // entities get added to each other's opponent lists and action
        // states transition to sword combat. The original game reads
        // the persistent jump line selected while reconsidering the enemy approach;
        // it does not recompute the line from the target's current position.
        // The target may have moved away from the edge during the approach,
        // but the retained line still has to become
        // jump-line destination on swordfight entry.
        self.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::Engage(
            self.base
                .primary_target
                .expect("swordfight entry target presence was checked"),
        ));
        self.base.outbox.actor.enter_swordfight_jump_line = self.my_line_jump;

        // VIPs use a different remark variant.
        if self.is_vip {
            self.base.say(Remark::VipStartsCombat);
        } else {
            self.base.say(Remark::StartsCombat);
        }
        self.base.clear_emoticon();
        self.set_state_with_timer(AiState::Attacking, Substate::AttackingSwordfight, 20, ctx);
    }

    // -----------------------------------------------------------------------
    // End swordfight
    // -----------------------------------------------------------------------

    pub fn end_swordfight(&mut self, ctx: &AiContext) {
        // If the entity is still swordfighting, launch a QUIT_SWORDFIGHT
        // sequence element to clear the opponent list and transition
        // action state. We can't call the engine directly, so we set a
        // pending flag that the engine picks up after the AI tick.
        if !ctx.is_swordfighting {
            return;
        }
        self.base.outbox.actor.quit_swordfight = true;
    }
}

/// Accepted output of [`rider_charge_goal_geometry`].
pub(crate) struct RiderChargeGeometry {
    pub forward_dot: f32,
    pub sq_norm: f32,
    pub cos_alpha: f32,
    /// `vMeToHitPoint` — map-space vector from the rider to the hit point.
    pub me_to_hit: (f32, f32),
    /// `vMeToHitPointNormalized` — `me_to_hit / hit_norm_len`.
    pub hit_dir: (f32, f32),
    /// `fMeToHitPointNorm`.
    pub hit_norm_len: f32,
    /// `ptGoal` — charge destination past the hit point.
    pub goal: (f32, f32),
}

/// Rejection reasons, carrying the value each debug print reports.
pub(crate) enum RiderChargeReject {
    Behind { forward_dot: f32 },
    TooNear { norm: f32, sq_norm: f32 },
    ZeroOrthogonal { ortho_len: f32 },
    ZeroHitVector { hp_len: f32 },
    ZeroHitNorm { hit_norm_len: f32 },
}

/// Pure geometry core of rider attack destination selection.
/// during rider-charge setup.
///
/// The charge goal feeds the movement order verbatim, so this math is
/// save-observable to the last bit. Two shapes are easy to get wrong:
///
/// * The nose vector is computed from the facing sector with the
///   **default** aspect ratio `1.0` — the raw
///   stretched-space table entry. Applying `ASPECT_RATIO` and then
///   unapplying `INVERSE_ASPECT_RATIO` lands an ULP off and can flip the
///   forward half-plane test for boundary vectors.
/// * Both vector-scaling sites round their
///   scalar **once** before touching the components (`Set(k*mX, k*mY)`,
///   vector operation): `k1 = RIDER_CHARGE_LATERAL_DISTANCE /
///   fCosAlpha` and `k2 = fCosAlpha *
/// the normalized enemy vector. Distributing the multiply per component
///   (`n.x * 40.0 / cos`) double-rounds differently; nicouzouf
///   Savegame_047 Soldier51's frame-563 charge goal came out one ULP low
///   in Y that way, which shifted the spliced running-order goal, its
///   normalized increment, and every subsequent walk step.
///
/// The `f32::EPSILON` degenerate-input rejections have no Original
/// counterpart (it would divide by zero and assert in debug); they are
/// unreachable for the finite, >= 40-unit vectors that pass the earlier
/// gates.
pub(crate) fn rider_charge_goal_geometry(
    my_pos: (f32, f32),
    my_dir: u16,
    enemy_pos: (f32, f32),
) -> Result<RiderChargeGeometry, RiderChargeReject> {
    // vMeToEnemyStretchedY = ptEnemy - ptMe;  .mY *= INVERSE_ASPECT_RATIO
    let me_to_enemy_sy = (
        enemy_pos.0 - my_pos.0,
        (enemy_pos.1 - my_pos.1) * INVERSE_ASPECT_RATIO,
    );

    // Facing-sector vector — default aspect 1.0.
    let nose_sy = MapVec::from_sector_with_aspect(my_dir, 1.0);

    // Is the enemy before me?
    let forward_dot = nose_sy.dot(MapVec::new(me_to_enemy_sy.0, me_to_enemy_sy.1));
    if forward_dot < 0.0 {
        return Err(RiderChargeReject::Behind { forward_dot });
    }

    // fMeToEnemySquareNorm / fMeToEnemyNorm.
    let sq_norm = me_to_enemy_sy.0 * me_to_enemy_sy.0 + me_to_enemy_sy.1 * me_to_enemy_sy.1;
    let norm = sq_norm.sqrt();
    if norm < EnemyAi::RIDER_CHARGE_LATERAL_DISTANCE {
        return Err(RiderChargeReject::TooNear { norm, sq_norm });
    }

    // fCosAlpha = sqrt( 1.0f - RIDER_CHARGE_SQR_LATERAL_DISTANCE / fMeToEnemySquareNorm )
    let cos_alpha = (1.0 - EnemyAi::RIDER_CHARGE_SQR_LATERAL_DISTANCE / sq_norm).sqrt();

    // The clockwise normal with aspect 1.0 is (y, -x); normalize it next.
    let ortho = (me_to_enemy_sy.1, -me_to_enemy_sy.0);
    let ortho_len = (ortho.0 * ortho.0 + ortho.1 * ortho.1).sqrt();
    if ortho_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroOrthogonal { ortho_len });
    }
    let ortho_norm = (ortho.0 / ortho_len, ortho.1 / ortho_len);
    // operator*=: one rounded scalar, then k1 * component.
    let k1 = EnemyAi::RIDER_CHARGE_LATERAL_DISTANCE / cos_alpha;
    let ortho_scaled = (k1 * ortho_norm.0, k1 * ortho_norm.1);

    // vMeToHitPointStretchedY = vMeToEnemyStretchedY + orthogonal; Normalize().
    let hit_point_sy = (
        me_to_enemy_sy.0 + ortho_scaled.0,
        me_to_enemy_sy.1 + ortho_scaled.1,
    );
    let hp_len = (hit_point_sy.0 * hit_point_sy.0 + hit_point_sy.1 * hit_point_sy.1).sqrt();
    if hp_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroHitVector { hp_len });
    }
    let hp_norm = (hit_point_sy.0 / hp_len, hit_point_sy.1 / hp_len);
    // operator*=: one rounded scalar, then k2 * component.
    let k2 = cos_alpha * norm;
    let hp_scaled = (k2 * hp_norm.0, k2 * hp_norm.1);

    // vMeToHitPoint — reapply the aspect ratio to Y.
    let me_to_hit = (hp_scaled.0, hp_scaled.1 * ASPECT_RATIO);

    // Scale the normalized hit-point-to-goal vector by the charge-loop distance.
    // ptGoal = ptMe + vMeToHitPoint + vHitPointToGoal.
    let hit_norm_len = (me_to_hit.0 * me_to_hit.0 + me_to_hit.1 * me_to_hit.1).sqrt();
    if hit_norm_len < f32::EPSILON {
        return Err(RiderChargeReject::ZeroHitNorm { hit_norm_len });
    }
    let hit_dir = (me_to_hit.0 / hit_norm_len, me_to_hit.1 / hit_norm_len);
    let goal = (
        my_pos.0 + me_to_hit.0 + hit_dir.0 * EnemyAi::RIDER_CHARGE_LOOP_DISTANCE,
        my_pos.1 + me_to_hit.1 + hit_dir.1 * EnemyAi::RIDER_CHARGE_LOOP_DISTANCE,
    );

    Ok(RiderChargeGeometry {
        forward_dot,
        sq_norm,
        cos_alpha,
        me_to_hit,
        hit_dir,
        hit_norm_len,
        goal,
    })
}

#[cfg(test)]
mod tests;
#[test]
fn battle_target_multiplicity_stacks_duplicate_friend_claims_as_uword() {
    let mut multiplicity = std::collections::BTreeMap::from([(174, 0)]);

    increment_battle_target_multiplicity(&mut multiplicity, 174);
    increment_battle_target_multiplicity(&mut multiplicity, 174);

    assert_eq!(multiplicity[&174], 2);

    multiplicity.insert(174, u32::from(u16::MAX));
    increment_battle_target_multiplicity(&mut multiplicity, 174);

    assert_eq!(multiplicity[&174], 0);
}

#[test]
fn appended_battle_target_retains_global_multiplicity_after_personal_reset() {
    // Battle planning resets only the target already in its personal enemy
    // list. A target appended by a nearby friend keeps the shared counter.
    let mut decision = std::collections::BTreeMap::from([(343, 0)]);
    let global = std::collections::BTreeMap::from([(343, 4), (345, 1)]);

    seed_appended_battle_target_multiplicity(&mut decision, 343, &global);
    seed_appended_battle_target_multiplicity(&mut decision, 345, &global);

    assert_eq!(decision[&343], 0, "personal target stays reset");
    assert_eq!(decision[&345], 1, "appended target retains shared count");

    increment_battle_target_multiplicity(&mut decision, 345);
    assert_eq!(decision[&345], 2, "a live friend claim still stacks");
}

#[test]
fn appended_battle_target_observes_an_earlier_owners_serial_reset() {
    // Task #146: S131 resets PC101's shared 16-bit value before S178 appends that
    // target in a later owner slot. Re-deriving occupancy from live fighter
    // states would resurrect the stale claim and count it twice.
    let mut shared = std::collections::BTreeMap::from([(101, 1)]);
    shared.insert(101, 0);

    let mut later_decision = std::collections::BTreeMap::from([(100, 0)]);
    seed_appended_battle_target_multiplicity(&mut later_decision, 101, &shared);
    increment_battle_target_multiplicity(&mut later_decision, 101);
    increment_battle_target_multiplicity(&mut shared, 101);

    assert_eq!(later_decision[&101], 1);
    assert_eq!(shared[&101], 1, "later owners retain the serial mutation");
}

#[test]
fn failed_shot_proposal_resets_melee_multiplicity_before_observe_fallback() {
    // Task #134: two swordfighters claimed target 174 during
    // battle planning, but the archer's shot-target proposal reset the shared
    // counters before returning no shot. The ensuing Observe selector must
    // therefore see zero melee claims (plus only any live bow claims).
    let mut decision = std::collections::BTreeMap::from([(172, 1), (171, 0), (174, 2)]);

    rebuild_battle_target_multiplicity_for_shot(&mut decision, &[172, 171, 174], []);

    assert_eq!(
        decision,
        std::collections::BTreeMap::from([(171, 0), (172, 0), (174, 0)])
    );

    rebuild_battle_target_multiplicity_for_shot(&mut decision, &[172, 171, 174], [171, 171]);
    assert_eq!(decision[&171], 2, "bow claims are rebuilt after the reset");
    assert_eq!(decision[&174], 0, "stale melee claims remain cleared");
}

#[test]
fn battle_friend_claim_uses_primary_target_not_swordfight_opponent() {
    // Task #61/#134 control: both friends still had PC174 in their melee
    // opponent lists, while their live AI primary target had retargeted to
    // PC173. An earlier same-frame attack on enemy 174 must not
    // overwrite the later primary-target value used by battle decisions.
    let swordfight_opponent = 174;
    let stale_attack_enemy_claim = swordfight_opponent;
    let live_primary_target = 173;
    let mut multiplicity =
        std::collections::BTreeMap::from([(live_primary_target, 0), (swordfight_opponent, 0)]);

    for _ in 0..2 {
        let target = battle_friend_primary_target(
            AiState::Attacking,
            Some(AiEntityHandle::new(live_primary_target)),
        )
        .expect("attacking friend has a live primary target");
        assert_ne!(target, stale_attack_enemy_claim);
        increment_battle_target_multiplicity(&mut multiplicity, target);
    }

    assert_eq!(multiplicity[&live_primary_target], 2);
    assert_eq!(multiplicity[&swordfight_opponent], 0);
}

impl EnemyAi {
    fn execute_fight_decision(
        &mut self,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<std::ops::ControlFlow<bool, Decision>> {
        let target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED,
            env.ctx,
            env.tick,
            Some(target_multiplicity),
        );
        let Some(target) = target else {
            return Ok(std::ops::ControlFlow::Continue(Decision::Observe));
        };
        self.base.primary_target = Some(target);
        self.attack_enemy(target.get(), env)
            .map_err(|call| call.then(crate::ai::DutyTail::FinishBattleFightAfterAttack))?;
        unreachable!("attack operation returns to its engine caller")
    }

    fn execute_last_reserve_decision(
        &mut self,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { sim, ctx, tick, .. } = env;
        let target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            ctx,
            tick,
            Some(target_multiplicity),
        );
        self.base.primary_target = target;
        if ctx.self_action_state.is_sword() {
            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BattleProvoke, 0..4) == 0 {
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(crate::element::Command::Provoke);
            } else if let Some(target_pos) = self
                .find_fighter(target, tick)
                .map(|f| f.position)
                .or_else(|| ctx.entity_view(target).map(|view| view.position))
            {
                let d = target_pos.map_point() - ctx.position.map_point();
                let dir = vec_to_sector(d.x, d.y);
                self.base.outbox.actor.set_direction_instantly = Some(dir as i16);
            }
        } else {
            self.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::RaiseSword);
            self.base.outbox.actor.enter_swordfight_jump_line = None;
        }
        self.base.outbox.actor.set_focus(target);
        self.set_state_with_timer(AiState::Attacking, Substate::AttackingLastReserve, 50, ctx);
        std::ops::ControlFlow::Break(true)
    }

    fn execute_observe_decision(
        &mut self,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
        let target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            ctx,
            tick,
            Some(target_multiplicity),
        );
        self.base.primary_target = target;
        self.base.outbox.actor.set_focus(target);
        if self.combat_trainer {
            self.base.set_emoticon(EmoticonType::XMark);
            self.set_state_with_timer(
                AiState::Attacking,
                Substate::AttackingApproachToObserve,
                1,
                ctx,
            );
        } else {
            // DECISION_OBSERVE uses the swordfight-observer
            // courage distance, not the proud-observer constant,
            // and launches a 50-tick timer even while approaching
            // so observers keep reconsidering if the active
            // fighter drops or the formation changes.
            // Primary-target replacement selects from the persistent
            // Them list, which can include an opponent outside
            // the nearby-fighter snapshot. Original dereferences
            // that selected actor directly for Position().
            let target = target
                .expect("Observe decision requires a primary target")
                .get();
            let target_pos = ctx
                .entity_view(target)
                .unwrap_or_else(|| {
                    panic!(
                        "Observe target {} is absent from owner {}'s live entity view",
                        target, self.base.me
                    )
                })
                .position;
            self.base.seek_position = target_pos;
            let observe_distance = AiController::value_between(
                parameters_ai::OBSERVE_SWORDFIGHT_MAX_DISTANCE,
                parameters_ai::OBSERVE_SWORDFIGHT_MIN_DISTANCE,
                self.get_courage() as u8,
            );
            // Original issues approach movement before changing state. Keep the
            // movement in the state change's synchronous actor-effect
            // prefix so a preceding stop-all request and its walking
            // replacement settle before FilterAIEvent.
            let first_new_order = self.base.outbox.actor.orders.len();
            self.base
                .go_near(target_pos, observe_distance as i32, GotoFlags::empty(), ctx);
            if self.base.outbox.actor.orders.len() > first_new_order {
                // The original game constructs the nearby route before the
                // following emoticon, state-change, and timer updates
                // and inline unreachable-point test. Rust's path
                // construction is engine-owned, so suspend that
                // exact tail behind the movement actor boundary.
                let route_effects = std::mem::take(&mut self.base.outbox.actor);
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ActorEffects(route_effects));
                self.base.outbox.reentrant.battle_observe_completion_pending = true;
                self.base.outbox.reentrant.owner_work.push(
                    crate::ai::AiOwnerWork::ResumeBattleObserveAfterGoNear {
                        target,
                        target_position: target_pos,
                    },
                );
            } else {
                // Local approach fast exits already own their result,
                // so no engine round trip is required.
                self.resume_battle_observe_after_go_near(
                    target,
                    target_pos,
                    tick.avenger_wait_position_for(target),
                    ctx,
                );
            }
            // The typed continuation owns normal logging and the
            // roof-fallback early return in both paths.
            return std::ops::ControlFlow::Break(false);
        }
        std::ops::ControlFlow::Break(true)
    }

    pub(crate) fn resume_battle_shot_selection(
        &mut self,
        target: Option<AiEntityHandle>,
        old_substate: Substate,
        cover_shield_bearer: HumanHandle,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let ThinkEnv { ctx, .. } = env;
        if let Some(target) = target {
            self.base.primary_target = Some(target);
            self.base.outbox.actor.set_focus(target.get());
            // AIMING_TIME_FORMULA = (110 - shooting_ability) / 2.
            // Use the soldier's modified shooting ability
            // (with alcohol penalty) — *not* IQ — so the
            // bow-aim timer tracks `shooting`.
            if ctx.self_action_state.is_bow() {
                if self.base.current_substate == Substate::AttackingBowAiming {
                    self.set_state(AiState::Attacking, Substate::AttackingBowShooting);
                    self.shoot_arrow_at(target.get(), ctx);
                } else {
                    let aim_time =
                        ((110u32).saturating_sub(self.get_shooting_ability(ctx) as u32)) / 2;
                    self.set_state_with_timer(
                        AiState::Attacking,
                        Substate::AttackingBowAiming,
                        aim_time.max(5),
                        ctx,
                    );
                }
            } else {
                self.base.stop_all();
                self.set_state(AiState::Attacking, Substate::AttackingBowLoading);
                self.base
                    .outbox
                    .actor
                    .launch_commands
                    .push(if self.enemy_seen_below {
                        crate::element::Command::EquipBowDown
                    } else {
                        crate::element::Command::EquipBow
                    });
            }
        } else {
            // No valid target — fall back to observe
            let mut multiplicity = global.primary_target_multiplicity_scratch.clone();
            if let Some(decision) = self.execute_battle_decision(
                env,
                Decision::ArcherObserve,
                old_substate,
                cover_shield_bearer,
                &mut multiplicity,
                global,
            )? {
                self.base
                    .register_log_line(LogLineType::BattleDecision, decision as u16);
            }
            return Ok(());
        }
        self.base
            .register_log_line(LogLineType::BattleDecision, Decision::Shoot as u16);
        Ok(())
    }

    fn execute_look_for_help_decision(
        &mut self,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<std::ops::ControlFlow<bool, Decision>> {
        let ThinkEnv { ctx, tick, .. } = env;
        let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        self.base.primary_target = target;
        self.base.friends_are_alerted = true;
        self.alert_officer(crate::ai::OfficerAlertCaller::BattleLookForHelp)?;
        unreachable!("officer coordination transfers to the engine")
    }

    fn execute_alert_soldiers_decision(
        &mut self,
        env: ThinkEnv<'_>,
    ) -> crate::ai::AiFlow<std::ops::ControlFlow<bool, Decision>> {
        let ThinkEnv { ctx, tick, .. } = env;
        let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        self.base.primary_target = target;
        // The battle overview can become stale while membership is
        // rebuilt synchronously. Original treats a vanished target
        // exactly like a rejected officer attack command and falls
        // back to reserve; do not resolve the legal handle-0 sentinel
        // as a required entity view.
        let Some(target) = target else {
            tracing::warn!(
                me = self.base.me,
                "alert-soldiers decision lost its primary target; reserving instead"
            );
            return Ok(std::ops::ControlFlow::Continue(Decision::Reserve));
        };
        self.base.friends_are_alerted = true;
        // DECISION_ALERT_SOLDIERS issues officer attack commands,
        // NOT AlertSoldiers, with the live target position.
        let center = ctx
            .expect_entity_view(target, "alert-soldiers primary target")
            .position;
        Err(crate::ai::DutyCall {
            flags: crate::ai::DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::CommandSoldiersToAttack { center },
            after: Vec::new(),
        })
    }

    fn execute_run_for_new_arrows_decision(
        &mut self,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
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
        if self.base.primary_target.is_some() {
            let target_pos = tick
                .nearby_fighters
                .iter()
                .find(|f| Some(AiEntityHandle::new(f.handle)) == self.base.primary_target)
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

        // nearest-door search without a reference point. The same filter chain
        // as the civilian Panic flee: building doors only,
        // authorized for this NPC, skip the actor's own
        // building, distance by maximum norm with +500
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
            let mut best = None;
            let mut minimum_distance = u16::MAX;
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
                let dx = (door.point_out.x - ctx.position.x).abs();
                let dy = (door.point_out.y - ctx.position.y).abs();
                let distance = super::util::legacy_nearest_door_distance(
                    dx,
                    dy,
                    Some(door.sector_out) != my_sector_num,
                    door.layer_out != my_layer,
                );
                if distance < minimum_distance {
                    // Nearest-door selection rejects a Lacklandist's
                    // otherwise-best candidate when its interior
                    // already contains any PC. A rejected house
                    // does not update the running minimum.
                    let dangerous_house = ctx.is_hostile_to_player()
                        && global
                            .houses
                            .iter()
                            .find(|h| h.sector_index == door.sector_in as u32)
                            .is_some_and(|h| {
                                h.occupant_ids
                                    .iter()
                                    .any(|id| matches!(id, crate::element::EntityId::Pc(_)))
                            });
                    if !dangerous_house {
                        best = Some(door.position_in);
                        minimum_distance = distance;
                    }
                }
            }
            best
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
            return std::ops::ControlFlow::Continue(Decision::Cassos);
        }
        std::ops::ControlFlow::Break(true)
    }

    fn execute_too_proud_to_attack_decision(
        &mut self,
        old_substate: Substate,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
        // Stand back and observe from a comfortable distance
        // while lesser soldiers fight.
        let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        self.base.primary_target = target;
        // The original game queries the primary target's position, whose actor
        // semantics differ from the literal fighter position: a
        // target currently passing a door resolves to the
        // committed destination-side gate point.  Prefer the
        // full Position() snapshot when target selection retained
        // the target for which this tick was built.  A target
        // selected synchronously during this decision has no
        // equivalent door snapshot yet, so use its live entity
        // view rather than silently substituting our own point.
        let target = target.expect("TooProudToAttack requires a primary target");
        let target_pos = if Some(target) == tick.primary_target_snapshot_handle {
            tick.primary_target_position.unwrap_or_else(|| {
                panic!("TooProudToAttack target {target} has no Position() snapshot")
            })
        } else {
            ctx.entity_view(target)
                .unwrap_or_else(|| {
                    panic!("TooProudToAttack newly selected target {target} disappeared")
                })
                .position
        };
        let d = target_pos.map_point() - ctx.position.map_point();
        let distance = d.iso_norm(ASPECT_RATIO);

        if distance < parameters_ai::PROUD_OBSERVER_MIN_DISTANCE as f32 {
            // Too close — step back.
            if let Some(goal) = self.propose_good_step_back_goal(
                target_pos,
                parameters_ai::PROUD_OBSERVER_GOOD_DISTANCE,
                parameters_ai::PROUD_OBSERVER_MIN_DISTANCE,
                env,
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
                return std::ops::ControlFlow::Continue(Decision::Fight);
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
                self.base.face_entity(target, ctx);
                self.set_state_with_timer(
                    AiState::Attacking,
                    Substate::AttackingTooProudToAttack,
                    20,
                    ctx,
                );
            }
        } else {
            // Good distance — face and observe.
            self.base.face_entity(target, ctx);
            self.base.outbox.actor.set_focus(self.base.primary_target);
            self.set_state_with_timer(
                AiState::Attacking,
                Substate::AttackingTooProudToAttack,
                20,
                ctx,
            );
        }

        // Only on first battle decision entry.
        if old_substate == Substate::AttackingReactiontime
            || old_substate == Substate::AttackingReactiontimeRunning
        {
            if self.is_vip {
                self.base.say(Remark::VipProudDontFight);
            } else {
                self.base.say(Remark::ProudDontFight);
            }
        }
        std::ops::ControlFlow::Break(true)
    }

    fn execute_archer_step_back_decision(
        &mut self,
        old_substate: Substate,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
        // Archer steps back from enemy that's too close, then
        // re-evaluates.
        let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
        self.base.primary_target = target;
        // The target selected while choosing ArcherStepBack can
        // disappear before this execution-time reselection.
        // Original reports that lifecycle race and retries the
        // decision as Shoot, whose own no-target path falls back
        // to ArcherObserve.
        let Some(target) = target else {
            tracing::warn!(
                me = self.base.me,
                "archer step-back decision lost its primary target; shooting instead"
            );
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        };
        // The original game re-reads the primary target's position after
        // primary-target replacement. In particular, a door-passing
        // target contributes its committed gate side rather than
        // the raw interpolated fighter position.
        let enemy_pos = self.archer_enemy_position(target, ctx);
        self.base.seek_position = enemy_pos;
        if let Some(goal) = self.propose_good_step_back_goal(
            enemy_pos,
            parameters_ai::ARCHER_GOOD_DISTANCE,
            parameters_ai::ARCHER_MIN_DISTANCE,
            env,
            ASPECT_RATIO,
        ) {
            let debug_step_back = archer_step_back_lifecycle_debug_matches(
                ctx.frame,
                ctx.original_creation_order,
                self.base.me,
            );
            if debug_step_back {
                crate::ai_enemy::parity_trace::ArcherstepDecision {
                    frame: &(ctx.frame),
                    co: &(ctx.original_creation_order),
                    me: &(self.base.me),
                    owner_pos: &(ctx.position),
                    animation: &(ctx.self_animation),
                    action_state: &(ctx.self_action_state),
                    reached_done: &(ctx.self_animation_reached_action_done),
                    timer_running: &(self.base.timer_is_running),
                    timer_ring: &(self.base.when_does_timer_ring),
                    already_on_point: &(self.base.already_on_point),
                    old_substate: &(old_substate),
                    target: &(target),
                    enemy_pos: &(enemy_pos),
                    goal: &(goal),
                }
                .emit();
            }
            self.go_to(
                AiState::Attacking,
                Substate::AttackingArcherRetireFromCombat,
                goal,
                GotoFlags::RUN,
                ctx,
            );
            if debug_step_back {
                crate::ai_enemy::parity_trace::ArcherstepAfterGoto {
                    frame: &(ctx.frame),
                    co: &(ctx.original_creation_order),
                    me: &(self.base.me),
                    state: &(self.base.current_state),
                    substate: &(self.base.current_substate),
                    already_on_point: &(self.base.already_on_point),
                    couldnt_reachpoint: &(self.base.couldnt_reachpoint),
                    halt: &(self.base.outbox.actor.halt),
                    additional_halts: &(self.base.outbox.actor.additional_halts),
                    order_count: &(self.base.outbox.actor.orders.len()),
                }
                .emit();
            }
        } else {
            // Can't step back — fall back to shooting.
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        }
        std::ops::ControlFlow::Break(true)
    }

    fn execute_archer_observe_decision(
        &mut self,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
        let target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            ctx,
            tick,
            Some(target_multiplicity),
        );
        self.base.primary_target = target;
        self.base.outbox.actor.set_focus(target);

        if ctx.self_action_state.is_bow() {
            self.set_state_with_timer(AiState::Attacking, Substate::AttackingBowObserving, 50, ctx);
        } else {
            self.base.stop_all();
            self.base
                .outbox
                .actor
                .launch_commands
                .push(if self.enemy_seen_below {
                    crate::element::Command::EquipBowDown
                } else {
                    crate::element::Command::EquipBow
                });
            self.set_state(AiState::Attacking, Substate::AttackingBowObservingLoading);
        }
        std::ops::ControlFlow::Break(true)
    }

    fn execute_cover_behind_shield_bearer_decision(
        &mut self,
        cover_shield_bearer: HumanHandle,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv {
            ctx, tick, grid, ..
        } = env;
        // Run to cover position behind shield bearer.
        self.update_shield_bearer_before_me(Some(AiEntityHandle::new(cover_shield_bearer)));
        // Adopt the shield bearer's primary target.
        let Some(sb_snap) = self.find_fighter(cover_shield_bearer, tick) else {
            self.update_shield_bearer_before_me(None);
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        };
        self.base.primary_target = sb_snap.primary_target;

        // The original game's behind-shield-bearer position calculation returns false
        // when the shield bearer has no primary target.  Do not
        // invent a target position here: the failed cover decision
        // must flow through Shoot (and potentially ArcherObserve).
        if self.base.primary_target.is_none() {
            self.update_shield_bearer_before_me(None);
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        }
        if let Some(cover_pos) = self.compute_position_behind_shield_bearer(
            self.shield_bearer_before_me
                .expect("cover formation lost its shield bearer")
                .get(),
            env,
        ) {
            // The original game passes the seek position as the output
            // argument to the position calculation behind the shield bearer.
            // The candidate therefore becomes observable as soon
            // as that call succeeds, even when the following view
            // radius check rejects it and the decision falls back
            // to Shoot/ArcherObserve.
            self.base.seek_position = cover_pos;
            // Cover point must be within view radius of the
            // primary target, otherwise the archer can't see
            // the enemy from behind the shield bearer.
            let target_pos = self
                .find_fighter(self.base.primary_target, tick)
                .map(|f| f.position)
                .or_else(|| {
                    ctx.entity_view(self.base.primary_target)
                        .map(|view| view.position)
                })
                .unwrap_or_else(|| {
                    panic!(
                        "shield bearer target {:?} is missing from the live AI snapshot",
                        self.base.primary_target
                    )
                });
            let d = target_pos.map_point() - cover_pos.map_point();
            if crate::ai_enemy::battle_decision_debug_enabled() {
                crate::ai_enemy::parity_trace::CoverArm {
                    frame: &(ctx.frame),
                    me: &(self.base.me),
                    bearer: &(cover_shield_bearer),
                    cover: &(cover_pos),
                    target: &(self.base.primary_target),
                    target_pos: &(target_pos),
                    sq: &(d.square_norm()),
                    sq_view: &(ctx.sq_standard_view_radius),
                    grid: &(grid.is_some()),
                }
                .emit();
            }
            if d.square_norm() >= ctx.sq_standard_view_radius {
                // Cover point too far from target — fall back to shoot
                self.update_shield_bearer_before_me(None);
                return std::ops::ControlFlow::Continue(Decision::Shoot);
            }

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
                    return std::ops::ControlFlow::Continue(Decision::Shoot);
                }
            }
            // Tell the shield bearer to announce the formation.
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::Say {
                    target: cover_shield_bearer,
                    remark: Remark::ArchersBehindShieldBearers,
                });
        } else {
            if crate::ai_enemy::battle_decision_debug_enabled() {
                crate::ai_enemy::parity_trace::CoverArmNone {
                    frame: &(ctx.frame),
                    me: &(self.base.me),
                    bearer: &(cover_shield_bearer),
                    grid: &(grid.is_some()),
                }
                .emit();
            }
            // Can't compute position — give up cover attempt.
            self.update_shield_bearer_before_me(None);
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        }
        std::ops::ControlFlow::Break(true)
    }

    fn execute_run_to_archery_point_decision(
        &mut self,
        global: &mut AiGlobalState,
        env: ThinkEnv<'_>,
    ) -> std::ops::ControlFlow<bool, Decision> {
        let ThinkEnv { ctx, tick, .. } = env;
        // Run to the next waypoint on the archery path.
        if let Some(wp) = self.archery_path_get_waypoint(global) {
            // Remember enemy elevation for later bend decision
            self.enemy_had_this_elevation = self
                .find_fighter(self.base.primary_target, tick)
                .map(|f| f.elevation as u16)
                .unwrap_or(0);
            if wp.is_shooting_point {
                // Run directly to shooting point (final
                // sprint). Shooting-point selection writes the
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
            return std::ops::ControlFlow::Continue(Decision::Shoot);
        }
        std::ops::ControlFlow::Break(true)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
/// Decision-local aggregates handed from `battle_decisions` to the
/// decision-tree pieces.
#[derive(Clone, Copy)]
pub(crate) struct BattleDecisionInputs {
    pub(crate) friends_lower_company: u16,
    pub(crate) soldiers_lower_pride: bool,
    pub(crate) simple_soldiers_near: bool,
    pub(crate) min_square_enemy_distance: u32,
    pub(crate) num_enemies_i_can_see: usize,
    pub(crate) friends_nearer_to_enemy: u16,
}
