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

/// Keep the battle-side decision trace independently gated from the engine
/// context trace. This diagnostic is process-local and stderr-only, so its
/// disabled path cannot alter state, RNG consumption, or serialization.
fn archer_step_back_lifecycle_debug_matches(
    frame: u32,
    creation_order: Option<u32>,
    owner_handle: u32,
) -> bool {
    if std::env::var_os("PARITY_DEBUG_ARCHER_STEP_BACK_LIFECYCLE").is_none() {
        return false;
    }
    let parse = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for ARCHERSTEP diagnostic: {error}")
            })
        })
    };
    parse("PARITY_DEBUG_ARCHER_STEP_BACK_FRAME").is_none_or(|expected| expected == frame)
        && parse("PARITY_DEBUG_ARCHER_STEP_BACK_CREATION_ORDER")
            .is_none_or(|expected| Some(expected) == creation_order)
        && parse("PARITY_DEBUG_ARCHER_STEP_BACK_OWNER_HANDLE")
            .is_none_or(|expected| expected == owner_handle)
}

/// `ReconsiderEnemyApproach` uses raw `RHposition` map coordinates and stores
/// their Euclidean norm in a `UWORD`. This is deliberately different from the
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

/// Derive Original's `bAlertingSoldierNear` from the friends already admitted
/// to `mlistUs`. The admission walk has performed the authoritative 360-degree
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

/// Original `SquareDistance(primary_target)` compares the actors' literal
/// 3D sprite positions, stretches world Y, includes Z, and then truncates the
/// `FLOAT` result to `ULONG` before BattleDecisions compares friend distances.
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

/// Mirror `IncrementPrimaryTargetMultiplicity`: every nearby friend in the
/// broad swordfight family adds another `UNOCCUPIED_PREFERRED` penalty.
fn increment_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
) {
    let count = multiplicity.entry(target).or_insert(0);
    // Original stores this counter in a UWORD.
    *count = u32::from((*count as u16).wrapping_add(1));
}

/// Preserve the shared counter for a target appended after BattleDecisions'
/// reset pass. Original resets multiplicity only for the enemies already in
/// `mlistThem`; a nearby friend's previously unseen target retains its live
/// global value when it is inserted later in the same decision.
fn seed_appended_battle_target_multiplicity(
    multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
    target: HumanHandle,
    shared_multiplicity: &std::collections::BTreeMap<HumanHandle, u32>,
) {
    multiplicity
        .entry(target)
        .or_insert_with(|| shared_multiplicity.get(&target).copied().unwrap_or(0));
}

/// Mirror ProposeShotTarget's use of the actors' shared multiplicity scratch:
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

/// Return the live `GetPrimaryTarget()` claim used by BattleDecisions' friend
/// scan. This is deliberately independent of both the friend's swordfight
/// opponent list and any earlier `AttackEnemy` target recorded during the
/// same owner pass: later AI work can retarget `mpPrimaryTarget` while the
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

/// Original `GetFighter(myCamp, i)` preserves the camp registry's append
/// order, including PC/soldier interleaving. Keep the cheap pre-visibility
/// predicates here; the soldier AI-state switch belongs after the 360 gate.
fn battle_fighter_candidates(
    fighters: &[FighterSnapshot],
    me: HumanHandle,
) -> impl Iterator<Item = &FighterSnapshot> {
    fighters.iter().filter(move |fighter| {
        fighter.handle != me && fighter.is_friendly && fighter.is_able_to_fight
    })
}

#[track_caller]
fn battle_friend_detected_360(
    ctx: &AiContext,
    me: HumanHandle,
    friend: HumanHandle,
    friend_position_world: crate::coordinates::WorldPoint3D,
    friend_direction: u16,
    target: &crate::ai_entity_view::AiEntityView,
) -> bool {
    // RHElementActorNPC::IsDetecting360Degrees(actor) returns before even
    // issuing the sight query unless both elements are active and outside a
    // building. BattleDecisions can still be reached synchronously while a
    // phalanx member is inactive (for example during PLAY_ANIM_FROZEN), so
    // the fact that its AI handler is running does not imply this predicate.
    if !ctx.self_is_active || ctx.in_building || !target.active || target.in_building {
        return false;
    }
    let detection_point = crate::stealth::detection_point_world(
        friend_position_world,
        target.posture,
        friend_direction as i16,
        target.is_rider,
    );
    let detected = super::soldier_detects_detection_point_360(
        ctx.self_upright_eye_world,
        ctx.self_view_radius,
        ctx.in_building,
        detection_point,
        target.in_building,
        ctx.obstacle_list(),
    );
    let dx = detection_point.x - ctx.self_upright_eye_world.x;
    let dy = (detection_point.y - ctx.self_upright_eye_world.y)
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    let dz = detection_point.z - ctx.self_upright_eye_world.z;
    tracing::trace!(
        frame = ctx.frame,
        me,
        friend,
        viewer_in_building = ctx.in_building,
        viewer_radius = ctx.self_view_radius,
        viewer_x = ctx.self_upright_eye_world.x,
        viewer_y = ctx.self_upright_eye_world.y,
        viewer_z = ctx.self_upright_eye_world.z,
        friend_in_building = target.in_building,
        friend_posture = ?target.posture,
        friend_x = detection_point.x,
        friend_y = detection_point.y,
        friend_z = detection_point.z,
        sq_distance = dx * dx + dy * dy + dz * dz,
        sq_radius = (ctx.self_view_radius as f32).powi(2),
        detected,
        "BattleDecisions us-list 360 gate"
    );
    detected
}

#[track_caller]
fn sleeping_enemy_detected_360(
    ctx: &AiContext,
    enemy: &SleepingEnemyInfo,
    target: &crate::ai_entity_view::AiEntityView,
) -> bool {
    super::soldier_detects_target_360(
        ctx.position,
        ctx.elevation,
        ctx.self_is_rider,
        ctx.self_view_radius,
        ctx.in_building,
        enemy.position,
        target.elevation,
        target.posture,
        target.is_rider,
        target.direction as i16,
        target.in_building,
        ctx.obstacle_list(),
    )
}

impl EnemyAi {
    pub(super) fn enter_battle_reserve(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
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
        self.set_state(AiState::Attacking, Substate::AttackingReserve);
        self.base.launch_timer(50, ctx.frame);
    }

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
        sim: &crate::sim_rng::SimulationContext,
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

        // Original calls the ordinary GetNewPrimaryTarget here after
        // inserting the sleeping enemies. Reuse that path so target ranking
        // keeps its live GetPosition() geometry, isometric Y stretch, Z
        // component, UWORD truncation, and IsAllowedToAttack gate.
        let target_handle = self.get_new_primary_target(PrimaryTargetFlags::empty(), ctx, tick);
        if let Some(target_handle) = target_handle {
            let target_pos = ctx
                .entity_view(target_handle)
                .unwrap_or_else(|| {
                    panic!("sleeping primary target {target_handle} disappeared after selection")
                })
                .position;
            // SetState(Attacking, ApproachingSleepingEnemy) +
            // GoNear(target_pos, 20, RUN).
            self.base.primary_target = Some(target_handle);
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
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
    fn kill_nearby_sleeping_enemies(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Combat trainers and merry-man-forest fighters call
        // `ReturnToDuty()` first — note the quirk that the function then
        // *continues* and may still overwrite state with
        // `SUBSTATE_ATTACKING_APPROACHING_SLEEPING_ENEMY` below. We
        // mirror the behaviour exactly.
        if self.combat_trainer || self.is_merry_man_forest(ctx) {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }

        // Original performs IsDetecting360Degrees here, after the
        // unconscious/not-carried gates and only when the final battle
        // fallback is reached. The engine snapshot carries ordered fighter
        // candidates but must remain observer-neutral.
        let visible = tick
            .nearby_sleeping_enemies
            .iter()
            .filter(|enemy| {
                let target = ctx.entity_view(enemy.handle).unwrap_or_else(|| {
                    panic!(
                        "KillNearbySleepingEnemies candidate {} is absent from the AI entity view",
                        enemy.handle
                    )
                });
                sleeping_enemy_detected_360(ctx, enemy, target)
            })
            .cloned()
            .collect::<Vec<_>>();

        self.approach_sleeping_enemies(sim, &visible, ctx, tick);
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
                if let Some(target) = target {
                    self.base.primary_target = Some(target);
                    self.attack_enemy(target.get(), None, ctx, tick, None);
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
        self.base.outbox.actor.look_sidewards = Some(LookDirection::Left);
    }

    // -----------------------------------------------------------------------
    // MakeBattlePredecisions — offensive or defensive?
    // -----------------------------------------------------------------------

    pub fn make_battle_predecisions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> Decision {
        // Archers with no ammo or already swordfighting → defensive.
        if self.is_archer() && (ctx.remaining_arrows == 0 || ctx.is_swordfighting) {
            return Decision::PredecisionDefensive;
        }

        // Already fleeing → defensive
        if self.base.current_state == AiState::Fleeing {
            return Decision::PredecisionDefensive;
        }

        // --------- US ---------
        // Original walks the persistent mlistUs at this exact call site.
        // Deriving the aggregate while constructing generic tick snapshots
        // both made stale list assumptions and used to trigger eager LOS.
        let mut us_points = 0_u32;
        let mut there_is_an_officer = false;
        for &friend_handle in &self.base.list_us {
            let friend = ctx.entity_view(friend_handle).unwrap_or_else(|| {
                panic!(
                    "MakeBattlePredecisions us-list member {} is absent from the AI entity view",
                    friend_handle
                )
            });
            if friend.is_pc {
                us_points = us_points.saturating_add(100);
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
                            "MakeBattlePredecisions soldier {} is absent from camp_soldiers",
                            friend_handle
                        )
                    });
                (soldier.pride, soldier.rank)
            };
            us_points = us_points.saturating_add(100 + u32::from(pride));
            there_is_an_officer |= friend_handle != self.base.me && rank == ProfileRank::Officer;
        }

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

        // Wounded soldiers are more pessimistic. Both operands are read
        // live off the element: the AI-side `old_life_points` snapshot
        // trails the real value by an entire damage exchange, and
        // `initial_life_points` is the spawn value, not the profile
        // maximum, so a soldier at 20/120 was scoring as unhurt.
        let max_lp = ctx.self_max_life_points.max(1);
        let cur_lp = ctx.self_life_points;
        if cur_lp < max_lp {
            odds = (odds as i32 * cur_lp as i32 / max_lp as i32) as i16;
        }

        // Officer nearby bonus (multiplicative): with OFFICER_ODDS_BONUS
        // = 30, soldiers with an officer nearby almost always choose
        // offensive behaviour.
        if self.get_rank() == ProfileRank::Soldier && there_is_an_officer {
            odds = (odds as i32 * combat::OFFICER_ODDS_BONUS).min(i16::MAX as i32) as i16;
        }

        self.old_odds = odds;

        // Decision based on odds and courage.
        let courage = self.get_courage();
        if odds < (50 - courage as i16 / 2)
            && crate::sim_rng::u16(sim, crate::sim_rng::RngSite::BattleCourage, 0..100) > courage
        {
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
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        if ctx.entity_view(self.base.me).is_none() {
            // An earlier actor slot can remove this soldier after its timer
            // tail was admitted. There is no live owner on which to apply a
            // tactical decision.
            // TODO: cancel admitted owner tails at the removal boundary.
            tracing::warn!(
                me = self.base.me,
                "BattleDecisions skipped after owner left the live entity view"
            );
            return;
        }
        // BattleDecisions does not use FillListWithAllNearFighters. Original
        // scans the complete same-camp fighter registry and gates each entry
        // with the owner's IsDetecting360Degrees (whose radius is profile /
        // posture dependent and may exceed the 500-unit swordfight radius).
        // Rebuild the decision-only aggregate fields from that complete camp
        // snapshot, leaving `nearby_fighters` radius semantics untouched for
        // combat-position and swordfight callers.
        let mut battle_tick = tick.clone();
        battle_tick.friends_lower_company = 0;
        battle_tick.soldiers_lower_pride = false;
        battle_tick.friends_nearer_to_enemy = 0;
        battle_tick.us_battle_points = 100 + self.soldier_profile_pride as u32;
        battle_tick.has_officer_nearby = false;
        battle_tick.simple_soldiers_near = false;
        let debug_them = super::them_lifecycle_debug_matches(ctx);

        self.base.list_us.clear();
        self.base.list_us.push(self.base.me);
        // GetFighter(myCamp, i) walks the camp's single append-only fighter
        // registry. PCs and soldiers therefore have to remain interleaved:
        // every admitted candidate performs an opaque visibility query, and
        // changing that query order changes the FastFind visibility cache.
        for fighter in battle_fighter_candidates(&battle_tick.fighter_registry, self.base.me) {
            let target = ctx.entity_view(fighter.handle).unwrap_or_else(|| {
                panic!(
                    "BattleDecisions camp fighter {} is absent from the AI entity view",
                    fighter.handle
                )
            });
            let (position_world, direction) = if fighter.is_soldier {
                let friend = battle_tick
                    .camp_soldiers
                    .iter()
                    .find(|friend| friend.handle == fighter.handle)
                    .unwrap_or_else(|| {
                        panic!(
                            "able camp soldier {} is absent from camp_soldiers",
                            fighter.handle
                        )
                    });
                (friend.position_world, friend.direction)
            } else {
                (target.detection_position_world, target.direction)
            };
            // Original evaluates `mpMe->IsDetecting360Degrees(pHuman)` here,
            // after the cheap able-to-fight gate and before the soldier-state
            // switch, and only when
            // BattleDecisions actually runs. Do not use the historical
            // snapshot bit: populating it eagerly issued O(N²) opaque-LOS
            // queries on every RefreshDetection pass and changed both trace
            // ordering and the visibility cache before any battle decision.
            if !battle_friend_detected_360(
                ctx,
                self.base.me,
                fighter.handle,
                position_world,
                direction,
                target,
            ) {
                continue;
            }
            if fighter.is_pc {
                self.base.list_us.push(fighter.handle);
                if self.company_number > 0 {
                    battle_tick.friends_lower_company =
                        battle_tick.friends_lower_company.saturating_add(1);
                }
                continue;
            }
            let friend = battle_tick
                .camp_soldiers
                .iter()
                .find(|friend| friend.handle == fighter.handle)
                .expect("soldier metadata was resolved above");
            if !matches!(
                friend.ai_state,
                AiState::Default | AiState::Wondering | AiState::Seeking | AiState::Attacking
            ) {
                continue;
            }
            if debug_them {
                eprintln!(
                    "[THEM frame={} co={:?} me={} phase=battle_friend_after_360 friend={} state={:?} substate={:?} primary_target={:?}]",
                    ctx.frame,
                    ctx.original_creation_order,
                    self.base.me,
                    friend.handle,
                    friend.ai_state,
                    friend.ai_substate,
                    friend.primary_target,
                );
            }
            self.base.list_us.push(friend.handle);
            if self.company_number > friend.company_number
                && (self.base.current_substate == Substate::AttackingReactiontime
                    || friend.ai_state == AiState::Attacking)
            {
                battle_tick.friends_lower_company =
                    battle_tick.friends_lower_company.saturating_add(1);
            }
            battle_tick.soldiers_lower_pride |= self.soldier_profile_pride > friend.pride;
            battle_tick.us_battle_points += 100 + friend.pride as u32;
            battle_tick.simple_soldiers_near |= friend.rank == ProfileRank::Soldier;
            battle_tick.has_officer_nearby |= friend.rank == ProfileRank::Officer;
        }
        let tick = &battle_tick;

        // Original `BattleDecisions` snapshots `mCurrentSubstate` into a
        // stack-local `oldSubstate` before performing any decision work.
        // Do not use `self.previous_substate` here: that is the unrelated,
        // serialized `mPreviousSubstate` used by the Charly-reunion flow.
        let old_substate = self.base.current_substate;
        tracing::trace!(
            me = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            "battle_decisions: entry"
        );
        // Focus(NULL) at BattleDecisions entry. The decision tree will
        // re-focus on a freshly chosen primary target later (via
        // `pending_focus`) if it picks Fight / Shoot / etc.
        self.base.outbox.actor.set_unfocus();

        // BattleDecisions consumes the persistent Them list. Original only
        // rebuilds that list at explicit perception/state-machine call sites
        // (for example EVENT_VIEW and TooProud entry); it does not refresh it
        // here. This matters when a deferred timer snapshot contains fewer
        // enemies than the last view event.
        self.list_them.retain(|&h| h != 0); // basic cleanup

        if debug_them {
            eprintln!(
                "[THEM frame={} co={:?} me={} phase=battle_entry list={:?}]",
                ctx.frame, ctx.original_creation_order, self.base.me, self.list_them,
            );
        }

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

        // BattleDecisions owns a fresh, local multiplicity calculation in
        // the original. It first resets every enemy currently in mlistThem,
        // then increments targets claimed by every nearby swordfighting-family
        // friend and finally ensures every enemy already in a swordfight has
        // at least one claimant. The engine-wide snapshot is deliberately not
        // equivalent: it still contains claims from actors outside this
        // decision's rebuilt us/them lists.
        let mut decision_target_multiplicity = std::collections::BTreeMap::new();
        for &enemy in &self.list_them {
            decision_target_multiplicity.insert(enemy, 0_u32);
            global.primary_target_multiplicity_scratch.insert(enemy, 0);
        }
        // Original chooses the primary target from the persistent personal
        // Them list before walking nearby friends and appending the enemies
        // they are attacking. Those appended entries broaden later tactical
        // scans, but must not retroactively replace this decision's primary
        // target merely because one is nearer.
        self.base.primary_target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::empty(),
            ctx,
            tick,
            Some(&decision_target_multiplicity),
        );
        if super::primary_swap_debug_enabled()
            && super::primary_swap_debug_matches(ctx.frame, self.base.me)
        {
            eprintln!(
                "[PRIMARY_SWAP frame={} co={:?} owner={} phase=battle_primary_selected list_them={:?} selected={:?}]",
                ctx.frame,
                ctx.original_creation_order,
                self.base.me,
                self.list_them,
                self.base.primary_target,
            );
        }

        // Continue the same Original friend loop after primary-target
        // selection: attacking friends already committed to a swordfight
        // always count as nearer; other attacking friends count when their
        // position is closer to our chosen target than ours is.
        let mut friends_nearer_to_enemy = 0_u16;
        if let Some(target) = ctx.entity_view(self.base.primary_target) {
            // Original deliberately mixes two position APIs here.  The
            // reference distance is `SquareDistance(primary_target)`, which
            // compares the actors' literal 3D sprite positions with the
            // isometric Y stretch, includes Z, and truncates to ULONG. Each
            // friend is then compared through
            // `Position(friend) - Position(primary_target)`: those calls
            // apply the committed door-side override and use the raw map
            // norm. Projecting the literal positions to map space before the
            // reference comparison can substantially enlarge the threshold
            // when the actors stand at different elevations.
            let owner = ctx.entity_view(self.base.me).unwrap_or_else(|| {
                panic!(
                    "BattleDecisions owner {} is absent from its live entity view",
                    self.base.me
                )
            });
            let my_target_sq = battle_owner_target_square_distance(
                owner.detection_position_world,
                target.detection_position_world,
            );
            for friend in &tick.camp_soldiers {
                if !self.base.list_us.contains(&friend.handle) {
                    continue;
                }
                let Some(_friend_target) =
                    battle_friend_primary_target(friend.ai_state, friend.primary_target)
                else {
                    continue;
                };
                if super::util::is_any_swordfight_substate(friend.ai_substate as u32) {
                    friends_nearer_to_enemy = friends_nearer_to_enemy.saturating_add(1);
                    continue;
                }
                let friend_position = ctx
                    .entity_view(friend.handle)
                    .unwrap_or_else(|| {
                        panic!(
                            "BattleDecisions friend {} disappeared from the AI entity view",
                            friend.handle
                        )
                    })
                    .position;
                let target_position = target.position;
                if battle_friend_is_nearer(friend_position, target_position, my_target_sq) {
                    friends_nearer_to_enemy = friends_nearer_to_enemy.saturating_add(1);
                }
            }
        }

        // Walk same-camp soldiers in STATE_ATTACKING and inject their
        // primary target into list_them so we hunt where they are
        // fighting. Skip self, missing primary_target, and anything
        // already in our list. The injection happens AFTER
        // num_enemies_i_can_see is captured.
        {
            let me = self.base.me;
            let mut friend_seen: Vec<HumanHandle> = Vec::new();
            for cs in &tick.camp_soldiers {
                // Original performs this inside the same friend loop that
                // first requires IsDetecting360Degrees and inserts the
                // soldier into mlistUs. A distant or occluded attacking ally
                // must not contribute its primary target merely because it
                // exists in the camp snapshot.
                if cs.handle == me || !self.base.list_us.contains(&cs.handle) {
                    continue;
                }
                // The owner-local world view samples earlier soldiers' live
                // AI controllers. Use that strict `GetPrimaryTarget()` value;
                // an earlier AttackEnemy claim can already be stale after a
                // later BattleDecisions retarget in the same owner envelope.
                let Some(target) = battle_friend_primary_target(cs.ai_state, cs.primary_target)
                else {
                    continue;
                };
                tracing::trace!(
                    frame = ctx.frame,
                    me,
                    friend = cs.handle,
                    friend_state = ?cs.ai_state,
                    friend_substate = ?cs.ai_substate,
                    snapshot_target = ?cs.primary_target,
                    target,
                    already_listed = self.list_them.contains(&target),
                    "BattleDecisions friend-seen Them injection candidate"
                );
                if target == me {
                    continue;
                }
                // The initial Them entries were reset to zero above. A
                // target introduced only by this friend was not part of
                // Original's reset loop, so retain its shared counter before
                // applying this decision's possible increment.
                seed_appended_battle_target_multiplicity(
                    &mut decision_target_multiplicity,
                    target,
                    &global.primary_target_multiplicity_scratch,
                );
                if super::util::is_any_swordfight_substate(cs.ai_substate as u32) {
                    increment_battle_target_multiplicity(&mut decision_target_multiplicity, target);
                    increment_battle_target_multiplicity(
                        &mut global.primary_target_multiplicity_scratch,
                        target,
                    );
                }
                // The Them list rejects duplicates on insertion, so a target
                // an earlier entry or friend already contributed is dropped
                // here rather than appended a second time. A same-camp target
                // is deliberately not filtered: the cleanup pass below is what
                // removes friends from the list.
                if self.list_them.contains(&target) || friend_seen.contains(&target) {
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
        // The same pass measures the nearest surviving enemy.
        let owner_world = ctx
            .entity_view(self.base.me)
            .unwrap_or_else(|| {
                panic!(
                    "BattleDecisions owner {} is absent from its live entity view",
                    self.base.me
                )
            })
            .detection_position_world;
        let mut min_square_enemy_distance = u32::MAX;
        let mut unconscious_enemies_from_them = Vec::new();
        {
            let mut idx = 0;
            while idx < self.list_them.len() {
                let h = self.list_them[idx];
                let (drop_entry, decrement_visible_count) = match ctx.entity_view(h) {
                    Some(view) => {
                        let is_friend = view.camp == ctx.camp;
                        if !is_friend && !view.is_dead && view.is_unconscious && !view.is_carried {
                            // Original builds listUnconsciousEnemies from
                            // entries removed from the persistent mlistThem
                            // during this exact cleanup pass.  A fresh
                            // detection snapshot is not equivalent: an enemy
                            // can stop being detectable as soon as it falls
                            // unconscious while its authoritative Them-list
                            // entry remains until BattleDecisions consumes it.
                            unconscious_enemies_from_them.push(crate::ai::SleepingEnemyInfo {
                                handle: h,
                                position: view.position,
                                is_pc: view.is_pc,
                                is_robin: view.is_robin,
                                is_vip: view.is_vip,
                            });
                        }
                        if !is_friend && view.is_able_to_fight {
                            // The minimum enemy distance is measured over
                            // the surviving Them list — which by now also
                            // holds the targets contributed by nearby
                            // attacking allies, so a fight raging next to
                            // us counts even when our own nearest enemy is
                            // far away. `SquareDistance` compares literal 3D
                            // sprite positions, stretches world Y, includes
                            // Z, and then truncates the result to `ULONG`.
                            let sq = battle_owner_target_square_distance(
                                owner_world,
                                view.detection_position_world,
                            );
                            if sq < min_square_enemy_distance {
                                min_square_enemy_distance = sq;
                            }
                        }
                        if !is_friend
                            && view.is_able_to_fight
                            && view.is_swordfighting
                            && decision_target_multiplicity.get(&h).copied().unwrap_or(0) == 0
                        {
                            decision_target_multiplicity.insert(h, 1);
                            global.primary_target_multiplicity_scratch.insert(h, 1);
                        }
                        (
                            is_friend || !view.is_able_to_fight,
                            !is_friend && !view.is_able_to_fight,
                        )
                    }
                    None => {
                        tracing::warn!(
                            me = self.base.me,
                            target = h,
                            "battle_decisions: dropping them-list entry missing from entity view"
                        );
                        (true, true)
                    }
                };
                if drop_entry {
                    // Original only decrements the captured visible count in
                    // the non-friend `!IsAbleToFight()` arm. A stale friend
                    // is deleted by the separate `IsFriend()` arm without
                    // consuming that count (RHartificialmalignity.cpp,
                    // BattleDecisions cleanup), which can intentionally
                    // leave a positive visible count with an empty Them list.
                    if decrement_visible_count && idx < num_enemies_i_can_see {
                        num_enemies_i_can_see -= 1;
                    }
                    self.list_them.remove(idx);
                    continue;
                }
                idx += 1;
            }
        }

        if debug_them {
            eprintln!(
                "[THEM frame={} co={:?} me={} phase=battle_cleanup_after visible_count={} list={:?} unconscious={:?}]",
                ctx.frame,
                ctx.original_creation_order,
                self.base.me,
                num_enemies_i_can_see,
                self.list_them,
                unconscious_enemies_from_them
                    .iter()
                    .map(|enemy| enemy.handle)
                    .collect::<Vec<_>>(),
            );
        }

        if num_enemies_i_can_see == 0 {
            // No visible enemies. Ordering:
            //   combat_trainer → my_shooting_point → archer-leaning-out
            //   → friends-see-enemies (seek) → missed-PC → unconscious
            //   → kill_nearby_sleeping. archer-leaning-out MUST come
            //   before the seek-friends-enemies arm — an archer parked
            //   on a bend point with friend-seen enemies should hold
            //   the firing position, not run away to seek.
            if self.combat_trainer {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                    sim,
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
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
                // Original calls ForecastDestinationForIA unconditionally;
                // retaining an old seek position is not a valid fallback.
                self.refresh_missed_pc_forecast(sim, tick);
                self.seek_area(
                    sim,
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                    self.pc_gone_away_in_this_direction,
                    global,
                    ctx,
                    tick,
                );
            } else if !unconscious_enemies_from_them.is_empty() && !self.is_merry_man_forest(ctx) {
                // Enemies removed from the persistent Them list above are
                // unconscious and not carried — put them back, select one,
                // and walk up to finish them off.
                debug_assert!(self.list_them.is_empty());
                self.approach_sleeping_enemies(sim, &unconscious_enemies_from_them, ctx, tick);
            } else {
                // Final "there is literally nothing going on" fallback —
                // look for sleeping enemies anywhere within the 360°
                // detection radius and walk over to one.
                self.kill_nearby_sleeping_enemies(sim, ctx, tick);
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
            // Original never consumes this field. Although
            // `ForceNextBattleDecision` also stores `mbResetBattleDecision`,
            // BattleDecisions does not read that flag or clear the forced
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
                let predecision = self.make_battle_predecisions(sim, ctx, tick);
                decision = if self.combat_trainer || predecision == Decision::PredecisionDefensive {
                    Decision::Cassos
                } else {
                    Decision::Fight
                };
            }
        } else {
            // (1) Predecision: Offensive or defensive?
            let predecision = self.make_battle_predecisions(sim, ctx, tick);

            // Use engine-populated cached values for battle context.
            let friends_with_lower_company = tick.friends_lower_company;
            let soldiers_with_lower_pride = tick.soldiers_lower_pride;

            if self.combat_trainer {
                decision = Decision::Observe;
            } else if predecision == Decision::PredecisionOffensive {
                ////////// offensive decisions //////////////

                if self.is_archer() && self.base.blood_alcohol == 0 {
                    if crate::ai_enemy::battle_decision_debug_enabled() {
                        eprintln!(
                            "ARCHER_DECISION frame={} me={} tower={} sbb={:?} shooting_point={:?} too_near={} pos={:?} primary={:?} primary_pos={:?}",
                            ctx.frame,
                            self.base.me,
                            self.tower_guard,
                            self.shield_bearer_before_me,
                            self.my_shooting_point,
                            self.base.primary_target.is_some()
                                && self.archer_is_too_near_to_enemy(
                                    &ctx.position,
                                    self.base.primary_target,
                                    ctx,
                                    tick,
                                ),
                            ctx.position,
                            self.base.primary_target,
                            self.find_fighter(self.base.primary_target, tick)
                                .map(|f| f.position),
                        );
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
                    } else if self.shield_bearer_before_me.is_some() && self.base.blood_alcohol == 0
                    {
                        // Already paired with a shield bearer — check if
                        // we're still in cover or need to reposition.
                        if let Some(cover_pos) =
                            self.shield_bearer_cover_position(self.shield_bearer_before_me, tick)
                        {
                            let diff = pos_diff(&ctx.position, &cover_pos);
                            if max_norm(diff) < archer::COVER_POINT_TOLERANCE as f32 {
                                // Still in cover — shoot
                                decision = Decision::Shoot;
                            } else {
                                // Need to reposition behind shield bearer
                                cover_shield_bearer = self
                                    .shield_bearer_before_me
                                    .expect("active archer cover has no shield bearer")
                                    .get();
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
                    } else if min_square_enemy_distance < combat::MIN_SQUARE_RESERVE_DISTANCE as u32
                    {
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
                    && self.is_too_proud_to_attack(ctx, tick, Some(&decision_target_multiplicity))
                {
                    // Too proud to fight alongside commoners.
                    decision = Decision::TooProudToAttack;
                } else if ctx.camp.is_hostile_to(crate::element::Camp::Royalists)
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
            primary_target = ?self.base.primary_target,
            num_enemies_i_can_see,
            friends_nearer_to_enemy,
            soldiers_lower_pride = tick.soldiers_lower_pride,
            friends_lower_company = tick.friends_lower_company,
            "battle_decisions: chose decision"
        );
        if crate::ai_enemy::battle_decision_debug_enabled() {
            eprintln!(
                "BATTLE_DECISION frame={} me={} decision={:?} old_substate={:?} primary={:?} seen={} friends_nearer={}",
                ctx.frame,
                self.base.me,
                decision,
                old_substate,
                self.base.primary_target,
                num_enemies_i_can_see,
                friends_nearer_to_enemy
            );
        }
        // Carry out decision (with possible fallback loop). The Observe
        // arm's avenger-on-roof fallback returns from the whole routine
        // before the log line is registered; every other path logs.
        if self.execute_battle_decision(
            sim,
            decision,
            old_substate,
            cover_shield_bearer,
            &mut decision_target_multiplicity,
            global,
            ctx,
            tick,
            grid,
        ) {
            self.base
                .register_log_line(LogLineType::BattleDecision, decision as u16);
        }
    }

    fn refresh_missed_pc_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        tick: &AiPerTickData,
    ) {
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
    /// LookForHelp registers exactly one final decision after AlertOfficer's
    /// route result is known. Every other path returns `true`.
    fn execute_battle_decision(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        mut decision: Decision,
        old_substate: Substate,
        cover_shield_bearer: HumanHandle,
        target_multiplicity: &mut std::collections::BTreeMap<HumanHandle, u32>,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Allow up to 5 fallback decision changes to prevent infinite loops
        for _ in 0..5 {
            match decision {
                Decision::Fight => {
                    let target = self.get_new_primary_target_with_mult_override(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED,
                        ctx,
                        tick,
                        Some(target_multiplicity),
                    );
                    if let Some(target) = target {
                        self.base.primary_target = Some(target);
                        self.attack_enemy(target.get(), Some(&mut *global), ctx, tick, grid);
                        if self
                            .base
                            .outbox
                            .reentrant
                            .reconsider_approach_completion_pending
                        {
                            // AttackEnemy calls ReconsiderEnemyApproach.
                            // Original constructs that GoNear synchronously,
                            // so this couldn't-reach test runs only after its
                            // typed route continuation. Keep the enclosing
                            // decision loop on the same owner FIFO instead of
                            // prematurely accepting/logging DECISION_FIGHT.
                            self.base
                                .outbox
                                .reentrant
                                .owner_work
                                .push(crate::ai::AiOwnerWork::ResumeBattleFightAfterReconsider);
                            return false;
                        }
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
                    self.enter_battle_reserve_with_multiplicity(
                        ctx,
                        tick,
                        Some(target_multiplicity),
                    );
                }

                Decision::LastReserve => {
                    let target = self.get_new_primary_target_with_mult_override(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                        Some(target_multiplicity),
                    );
                    self.base.primary_target = target;
                    if ctx.self_action_state.is_sword() {
                        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BattleProvoke, 0..4)
                            == 0
                        {
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
                            let d = pos_diff(&target_pos, &ctx.position);
                            let dir = vec_to_sector(d.0, d.1);
                            self.base.outbox.actor.set_direction_instantly = Some(dir as i16);
                        }
                    } else {
                        self.base.outbox.actor.enter_swordfight =
                            Some(EnterSwordfightRequest::RaiseSword);
                        self.base.outbox.actor.enter_swordfight_jump_line = None;
                    }
                    self.base.outbox.actor.set_focus(target);
                    self.set_state(AiState::Attacking, Substate::AttackingLastReserve);
                    self.base.launch_timer(50, ctx.frame);
                }

                Decision::Observe => {
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
                        self.set_state(AiState::Attacking, Substate::AttackingApproachToObserve);
                        self.base.launch_timer(1, ctx.frame);
                    } else {
                        // DECISION_OBSERVE uses the swordfight-observer
                        // courage distance, not the proud-observer constant,
                        // and launches a 50-tick timer even while approaching
                        // so observers keep reconsidering if the active
                        // fighter drops or the formation changes.
                        // GetNewPrimaryTarget selects from the persistent
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
                        // Original issues GoNear before SetState. Keep the
                        // movement in SetState's synchronous actor-effect
                        // prefix so a preceding StopAll and its walking
                        // replacement settle before FilterAIEvent.
                        let first_new_order = self.base.outbox.actor.orders.len();
                        self.base.go_near(
                            target_pos,
                            observe_distance as i32,
                            GotoFlags::empty(),
                            ctx,
                        );
                        if self.base.outbox.actor.orders.len() > first_new_order {
                            // Original GoNear constructs the route before the
                            // following SetEmoticon/SetState/timer statements
                            // and inline mbCouldntReachpoint test. Rust's path
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
                            // Local GoNear fast exits already own their result,
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
                        return false;
                    }
                }

                Decision::Shoot => {
                    if ctx.remaining_arrows == 0 {
                        decision = Decision::RunForNewArrows;
                        continue;
                    }
                    // Pick best shot target.
                    let target = self.propose_shot_target(sim, ctx, tick);
                    // ProposeShotTarget uses the actors' shared multiplicity
                    // scratch field: it resets every current Them entry, then
                    // rebuilds claims from nearby friends in bow substates.
                    // Preserve that side effect for a failed-shot fallback to
                    // Observe/Fight, which immediately reuses the field in
                    // GetNewPrimaryTarget.
                    let bow_targets: Vec<_> = self
                        .base
                        .list_us
                        .iter()
                        .copied()
                        .filter(|&friend_handle| friend_handle != self.base.me)
                        .filter_map(|friend_handle| {
                            let friend = self.find_fighter(friend_handle, tick).unwrap_or_else(|| {
                                panic!(
                                    "friend {friend_handle} in list_us is absent from fighter snapshot"
                                )
                            });
                            (friend.is_soldier
                                && matches!(
                                    friend.current_substate,
                                    x if x == Substate::AttackingBowShooting as u32
                                        || x == Substate::AttackingBowLoading as u32
                                        || x == Substate::AttackingBowAiming as u32
                                )
                                && friend.primary_target.is_some())
                                .then(|| friend.primary_target.map(AiEntityHandle::get))
                                .flatten()
                        })
                        .collect();
                    rebuild_battle_target_multiplicity_for_shot(
                        target_multiplicity,
                        &self.list_them,
                        bow_targets.iter().copied(),
                    );
                    for target in self.list_them.iter().chain(bow_targets.iter()) {
                        let count = target_multiplicity.get(target).copied().unwrap_or(0);
                        global
                            .primary_target_multiplicity_scratch
                            .insert(*target, count);
                    }
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
                                self.shoot_arrow_at(target.get(), ctx, tick);
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
                        // Original: `(rand() & 1) ? CASSOS : PANIC`.
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
                        self.begin_cassos_panic(
                            target
                                .expect("Cassos decision requires a primary target")
                                .get(),
                            ctx,
                            tick,
                        );
                    }
                }

                Decision::LookForHelp => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    self.base.friends_are_alerted = true;
                    // Original immediately evaluates Position(mpPrimaryTarget)
                    // for AlertOfficer. The selected pointer must still
                    // resolve in the live entity view; neither cached fighter
                    // geometry nor an older seek point can substitute for it.
                    let center = ctx
                        .expect_entity_view(
                            target.expect("LookForHelp requires a primary target"),
                            "LookForHelp primary target",
                        )
                        .position;
                    // Original derives this while building `mlistUs`; reuse
                    // that admission result rather than issuing a second set
                    // of 360-degree visibility queries.
                    let alerting_soldier_near = has_nearby_alerting_soldier(
                        self.base.me,
                        &self.base.list_us,
                        tick.camp_soldiers
                            .iter()
                            .map(|cs| (cs.handle, cs.ai_substate)),
                    );
                    if alerting_soldier_near || !self.alert_officer(sim, center, 0, ctx, tick) {
                        decision = Decision::Cassos;
                        continue;
                    } else {
                        // AlertOfficer calls GoNear synchronously. Its route
                        // construction can consume the paired random building
                        // exit wait before control returns here to draw the
                        // Cassos/Panic remark. Close the GoNear actor prefix
                        // and resume this statement at the owner boundary so
                        // Rust preserves that call-stack ordering.
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                &mut self.base.outbox.actor,
                            )),
                        );
                        self.base.outbox.reentrant.look_for_help_completion_pending = true;
                        self.base
                            .outbox
                            .reentrant
                            .owner_work
                            .push(crate::ai::AiOwnerWork::ResumeBattleLookForHelpAfterAlertOfficer);
                        // The continuation owns the single final battle log:
                        // LookForHelp after success, Cassos after route failure.
                        return false;
                    }
                }

                Decision::AlertSoldiers => {
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    // The battle overview can become stale while membership is
                    // rebuilt synchronously. Original treats a vanished target
                    // exactly like a rejected CommandSoldiersToAttack and falls
                    // back to reserve; do not resolve the legal handle-0 sentinel
                    // as a required entity view.
                    let Some(target) = target else {
                        tracing::warn!(
                            me = self.base.me,
                            "alert-soldiers decision lost its primary target; reserving instead"
                        );
                        decision = Decision::Reserve;
                        continue;
                    };
                    self.base.friends_are_alerted = true;
                    // DECISION_ALERT_SOLDIERS calls CommandSoldiersToAttack,
                    // NOT AlertSoldiers, with the live target position.
                    let center = ctx
                        .expect_entity_view(target, "alert-soldiers primary target")
                        .position;
                    match self.command_soldiers_to_attack(center, global, grid, ctx, tick) {
                        super::alert::CommandSoldiersStart::Pending => return true,
                        super::alert::CommandSoldiersStart::Rejected => {
                            decision = Decision::Reserve;
                            continue;
                        }
                    }
                }

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
                    if !self.run_and_alert_soldiers(center, ctx, tick, global) {
                        decision = Decision::Cassos;
                        continue;
                    } else {
                        // Random Cassos/Panic remark.
                        // Original: `(rand() & 1) ? CASSOS : PANIC`.
                        if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
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
                    // `GetNewPrimaryTarget` can legally return the null handle
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
                            .find(|f| {
                                Some(AiEntityHandle::new(f.handle)) == self.base.primary_target
                            })
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
                            let distance = crate::ai::legacy_nearest_door_distance(
                                dx,
                                dy,
                                Some(door.sector_out) != my_sector_num,
                                door.layer_out != my_layer,
                            );
                            if distance < minimum_distance {
                                // GetNearestDoor rejects a Lacklandist's
                                // otherwise-best candidate when its interior
                                // already contains any PC. A rejected house
                                // does not update the running minimum.
                                let dangerous_house =
                                    ctx.camp.is_hostile_to(crate::element::Camp::Royalists)
                                        && global
                                            .houses
                                            .iter()
                                            .find(|h| h.sector_index == door.sector_in as u32)
                                            .is_some_and(|h| {
                                                h.occupant_ids.iter().any(|id| {
                                                    matches!(id, crate::element::EntityId::Pc(_))
                                                })
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
                    // Original calls Position(mpPrimaryTarget), whose actor
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
                                panic!(
                                    "TooProudToAttack newly selected target {target} disappeared"
                                )
                            })
                            .position
                    };
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
                            self.base.face_entity(target, ctx);
                            self.set_state(AiState::Attacking, Substate::AttackingTooProudToAttack);
                            self.base.launch_timer(20, ctx.frame);
                        }
                    } else {
                        // Good distance — face and observe.
                        self.base.face_entity(target, ctx);
                        self.base.outbox.actor.set_focus(self.base.primary_target);
                        self.set_state(AiState::Attacking, Substate::AttackingTooProudToAttack);
                        self.base.launch_timer(20, ctx.frame);
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
                }

                Decision::ArcherStepBack => {
                    // Archer steps back from enemy that's too close, then
                    // re-evaluates.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
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
                        decision = Decision::Shoot;
                        continue;
                    };
                    // Original re-reads Position(mpPrimaryTarget) after
                    // GetNewPrimaryTarget.  In particular, a door-passing
                    // target contributes its committed gate side rather than
                    // the raw interpolated fighter position.
                    let enemy_pos = self.archer_enemy_position(target, ctx);
                    self.base.seek_position = enemy_pos;
                    if let Some(goal) = self.propose_good_step_back_goal(
                        enemy_pos,
                        parameters_ai::ARCHER_GOOD_DISTANCE,
                        parameters_ai::ARCHER_MIN_DISTANCE,
                        ctx,
                        grid,
                        ASPECT_RATIO,
                    ) {
                        let debug_step_back = archer_step_back_lifecycle_debug_matches(
                            ctx.frame,
                            ctx.original_creation_order,
                            self.base.me,
                        );
                        if debug_step_back {
                            eprintln!(
                                "[ARCHERSTEP frame={} co={:?} me={} phase=decision old_substate={old_substate:?} target={target} owner_pos={:?} enemy_pos={enemy_pos:?} goal={goal:?} animation={:?} action_state={:?} reached_done={} timer_running={} timer_ring={} already_on_point={}]",
                                ctx.frame,
                                ctx.original_creation_order,
                                self.base.me,
                                ctx.position,
                                ctx.self_animation,
                                ctx.self_action_state,
                                ctx.self_animation_reached_action_done,
                                self.base.timer_is_running,
                                self.base.when_does_timer_ring,
                                self.base.already_on_point,
                            );
                        }
                        self.go_to(
                            AiState::Attacking,
                            Substate::AttackingArcherRetireFromCombat,
                            goal,
                            GotoFlags::RUN,
                            ctx,
                        );
                        if debug_step_back {
                            eprintln!(
                                "[ARCHERSTEP frame={} co={:?} me={} phase=after_goto state={:?} substate={:?} already_on_point={} couldnt_reachpoint={} halt={} additional_halts={} order_count={}]",
                                ctx.frame,
                                ctx.original_creation_order,
                                self.base.me,
                                self.base.current_state,
                                self.base.current_substate,
                                self.base.already_on_point,
                                self.base.couldnt_reachpoint,
                                self.base.outbox.actor.halt,
                                self.base.outbox.actor.additional_halts,
                                self.base.outbox.actor.orders.len(),
                            );
                        }
                    } else {
                        // Can't step back — fall back to shooting.
                        decision = Decision::Shoot;
                        continue;
                    }
                }

                Decision::ArcherObserve => {
                    let target = self.get_new_primary_target_with_mult_override(
                        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                        ctx,
                        tick,
                        Some(target_multiplicity),
                    );
                    self.base.primary_target = target;
                    self.base.outbox.actor.set_focus(target);

                    if ctx.self_action_state.is_bow() {
                        self.set_state(AiState::Attacking, Substate::AttackingBowObserving);
                        self.base.launch_timer(50, ctx.frame);
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
                }

                Decision::Menace => {
                    // Menace a PC in coma.
                    let target =
                        self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
                    self.base.primary_target = target;
                    let _target = target.expect("Menace decision requires a primary target");
                    self.set_state(AiState::Menacing, Substate::MenacingPcInComa);
                    self.base
                        .launch_timer(parameters_ai::AI_MENACING_PATIENCE as u32, ctx.frame);
                }

                Decision::CoverBehindShieldBearer => {
                    // Run to cover position behind shield bearer.
                    self.update_shield_bearer_before_me(cover_shield_bearer);
                    // Adopt the shield bearer's primary target.
                    let Some(sb_snap) = self.find_fighter(cover_shield_bearer, tick) else {
                        self.update_shield_bearer_before_me(0);
                        decision = Decision::Shoot;
                        continue;
                    };
                    self.base.primary_target = sb_snap.primary_target;

                    // Original ComputePositionBehindMyShieldBearer returns false
                    // when the shield bearer has no primary target.  Do not
                    // invent a target position here: the failed cover decision
                    // must flow through Shoot (and potentially ArcherObserve).
                    if self.base.primary_target.is_none() {
                        self.update_shield_bearer_before_me(0);
                        decision = Decision::Shoot;
                        continue;
                    }
                    if let Some(cover_pos) = self.compute_position_behind_shield_bearer(
                        self.shield_bearer_before_me
                            .expect("cover formation lost its shield bearer")
                            .get(),
                        ctx,
                        tick,
                        grid,
                    ) {
                        // Original passes `mposSeekPosition` as the output
                        // argument to ComputePositionBehindMyShieldBearer.
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
                        let d = pos_diff(&target_pos, &cover_pos);
                        if crate::ai_enemy::battle_decision_debug_enabled() {
                            eprintln!(
                                "COVER_ARM frame={} me={} bearer={} cover={:?} target={:?} target_pos={:?} sq={} sq_view={} grid={}",
                                ctx.frame,
                                self.base.me,
                                cover_shield_bearer,
                                cover_pos,
                                self.base.primary_target,
                                target_pos,
                                square_norm(d),
                                ctx.sq_standard_view_radius,
                                grid.is_some(),
                            );
                        }
                        if square_norm(d) >= ctx.sq_standard_view_radius {
                            // Cover point too far from target — fall back to shoot
                            self.update_shield_bearer_before_me(0);
                            decision = Decision::Shoot;
                            continue;
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
                                decision = Decision::Shoot;
                                continue;
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
                            eprintln!(
                                "COVER_ARM frame={} me={} bearer={} cover=None grid={}",
                                ctx.frame,
                                self.base.me,
                                cover_shield_bearer,
                                grid.is_some(),
                            );
                        }
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
                            .map(|f| f.elevation as u16)
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
        true
    }

    /// Resume the `DECISION_FIGHT` tail after its nested
    /// `ReconsiderEnemyApproach` route has settled.
    pub(crate) fn resume_battle_fight_after_reconsider(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if !self.base.couldnt_reachpoint {
            self.base
                .register_log_line(LogLineType::BattleDecision, Decision::Fight as u16);
            return;
        }

        // RHArtificialMalignity::BattleDecisions clears the failed fight
        // approach and loops directly into DECISION_OBSERVE. Rebuild the
        // local multiplicities from the live scratch counters retained by
        // this owner boundary; Observe's target selection reads them.
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
            sim,
            Decision::Observe,
            self.base.current_substate,
            0,
            &mut target_multiplicity,
            global,
            ctx,
            tick,
            None,
        );
        if completed_inline {
            self.base
                .register_log_line(LogLineType::BattleDecision, Decision::Observe as u16);
        }
    }

    /// Execute the two Original CASSOS Panic overloads after primary-target
    /// selection. A non-null `RHElementActorHuman*` is read through
    /// `Point(target)` at this call site; retaining an older seek point is not
    /// a valid substitute if the selected actor cannot be resolved. A null
    /// target deliberately calls the undirected `Panic(runs)` overload.
    fn begin_cassos_panic(&mut self, target: HumanHandle, ctx: &AiContext, _tick: &AiPerTickData) {
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

    /// Resume the statement immediately following `AlertOfficer`'s
    /// synchronous `GoNear` in `DECISION_LOOK_4_HELP`.
    ///
    /// Rust constructs cross-sector routes after releasing the AI borrow, so
    /// this tail must run at the owner boundary. In Original, a failed route
    /// is consumed by `AlertOfficer` itself and changes the decision to
    /// `CASSOS`; it is not delivered as `EVENT_COULDNT_REACHPOINT`.
    pub(crate) fn resume_battle_look_for_help_after_alert_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if !self.base.couldnt_reachpoint {
            if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
                self.base.say(Remark::Cassos);
            } else {
                self.base.say(Remark::Panic);
            }
            self.base
                .register_log_line(LogLineType::BattleDecision, Decision::LookForHelp as u16);
            return;
        }

        // AlertOfficer clears the latch before returning false. The enclosing
        // decision loop then executes the ordinary CASSOS arm.
        self.base.couldnt_reachpoint = false;
        if !self.is_merry_man_forest(ctx) || !self.merry_man_forest_cassos(ctx, global) {
            if crate::sim_rng::bool(sim, crate::sim_rng::RngSite::BattlePanicRemark) {
                self.base.say(Remark::Cassos);
            } else {
                self.base.say(Remark::Panic);
            }
            let target = self.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, ctx, tick);
            self.base.primary_target = target;
            self.begin_cassos_panic(
                target
                    .expect("Cassos continuation requires a primary target")
                    .get(),
                ctx,
                tick,
            );
        }
        self.base
            .register_log_line(LogLineType::BattleDecision, Decision::Cassos as u16);
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

        // Unconditional `mposSeekPosition = Position(pEnemy)`. When the
        // selected enemy is the target for which this tick was built, the
        // target-specific snapshot is the authoritative result of that exact
        // call (including door/carrier semantics). A generic fighter snapshot
        // may have been sampled at an earlier owner boundary and must not win
        // merely because it contains the same handle.
        let enemy_pos = if Some(AiEntityHandle::new(enemy)) == tick.primary_target_snapshot_handle {
            tick.primary_target_position
                .unwrap_or_else(|| panic!("AttackEnemy target {enemy} has no Position() snapshot"))
        } else {
            // BattleDecisions can select a target appended from a nearby
            // friend's live GetPrimaryTarget() after the dedicated target
            // snapshot was built. Original immediately evaluates
            // Position(pEnemy), including that actor's exact RHSector*.
            // The broad fighter snapshot is detection geometry sampled at an
            // earlier owner boundary and can retain only the duplicate public
            // sector number, so the live entity view must win here.
            ctx.entity_view(enemy)
                .map(|view| view.position)
                .or_else(|| {
                    tick.nearby_fighters
                        .iter()
                        .find(|fighter| fighter.handle == enemy)
                        .map(|fighter| fighter.position)
                })
                .unwrap_or_else(|| panic!("AttackEnemy target {enemy} disappeared"))
        };
        self.base.seek_position = enemy_pos;

        // primary_target then emoticon.
        self.base.primary_target = Some(AiEntityHandle::new(enemy));
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
        let debug_decision_path = super::decision_path_debug_enabled()
            && super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} co={:?} stage=reconsider_enter reachpoint={} distance_arg_bits={:08x} state={:?}/{:?} primary={:?} seek=({:08x},{:08x},sector={:?},level={}) rider={} couldnt={} already={} owner_work={:?}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                reachpoint,
                _distance_arg.to_bits(),
                self.base.current_state,
                self.base.current_substate,
                self.base.primary_target,
                self.base.seek_position.x.to_bits(),
                self.base.seek_position.y.to_bits(),
                self.base.seek_position.sector,
                self.base.seek_position.level,
                ctx.self_is_rider,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
            );
        }
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

        // Original reads `mpMe->GetStandardRangeSword()` from the actor's
        // live RHSword here and again for each GoNear tolerance.  Do not use
        // EnemyAi's compatibility cache: old replay/save snapshots can carry
        // a range written with the former zero-based weapon-id convention.
        let standard_sword_range = self
            .find_fighter(self.base.me, tick)
            .unwrap_or_else(|| {
                panic!(
                    "ReconsiderEnemyApproach owner {} missing from fighter registry",
                    self.base.me
                )
            })
            .sword_range_default;
        // sword_range = standard sword range + 10.
        let sword_range: f32 = (standard_sword_range + 10) as f32;
        let mut run_distance = self.compute_enemy_run_distance(standard_sword_range) as f32;
        let standard_sword_range = standard_sword_range as f32;

        let mut b_reconsider = false;

        // Target on another entity's shoulders: re-point `primary_target`
        // to the carrier so every downstream read (friend-swap
        // comparison, `Focus`, `BeginSwordfight`'s
        // `pending_enter_swordfight`) sees the carrier rather than the
        // carried entity. The reference does
        // `primary_target = primary_target.GetCarrier()`, persisting
        // across ticks because `primary_target` is a member.
        let target_snapshot_is_current =
            tick.primary_target_snapshot_handle == self.base.primary_target;
        let target_on_shoulders = if target_snapshot_is_current {
            matches!(
                tick.primary_target_posture,
                Some(crate::element::Posture::OnShoulders)
            )
        } else {
            ctx.entity_view(self.base.primary_target)
                .unwrap_or_else(|| {
                    panic!(
                        "ReconsiderEnemyApproach target {:?} disappeared after synchronous retarget",
                        self.base.primary_target
                    )
                })
                .posture
                == crate::element::Posture::OnShoulders
        };
        if target_on_shoulders {
            if !target_snapshot_is_current {
                // TODO: expose the carrier handle in AiEntityView so a target
                // changed synchronously to a carried human can be resolved
                // here without rebuilding the whole tick snapshot.
                panic!(
                    "ReconsiderEnemyApproach synchronously retargeted to carried human {:?}; carrier identity is unavailable",
                    self.base.primary_target
                );
            }
            let carrier_handle = tick.primary_target_carrier_handle.unwrap_or_else(|| {
                panic!(
                    "ReconsiderEnemyApproach target {:?} is on shoulders without a carrier snapshot",
                    self.base.primary_target
                )
            });
            self.base.primary_target = Some(carrier_handle);
        }

        // Position(primary_target) after the substitution resolves to
        // the carrier's position when the carry path fired.
        let live_target_pos = if target_snapshot_is_current {
            if target_on_shoulders && let Some(carrier) = tick.primary_target_carrier_position {
                carrier
            } else {
                tick.primary_target_position.unwrap_or_else(|| {
                    panic!(
                        "ReconsiderEnemyApproach target {:?} has no position snapshot",
                        self.base.primary_target
                    )
                })
            }
        } else {
            // Original reads Position(mpPrimaryTarget) after the timer/event
            // callback has synchronously changed that pointer. Resolve the
            // replacement handle through the shared per-frame entity view;
            // using `tick.primary_target_position` here couples the new
            // identity to the old target's coordinates.
            ctx.entity_view(self.base.primary_target)
                .unwrap_or_else(|| {
                    panic!(
                        "ReconsiderEnemyApproach target {:?} disappeared after synchronous retarget",
                        self.base.primary_target
                    )
                })
                .position
        };

        // This specific Original routine uses the raw map-coordinate norm and
        // truncates it through UWORD. Do not use the usual isometric Y stretch.
        let distance = reconsider_approach_distance(live_target_pos, ctx.position);

        // Line-jump for table swordfight. `RHArtificialMalignity::
        // ReconsiderEnemyApproach` (RHartificialmalignity.cpp:6746) evaluates
        // `mpMe->IsTableSwordfightNeeded( mpPrimaryTarget )` live against the
        // primary target as it stands on entry — after any synchronous
        // retarget by the calling decision, and before the friend-swap loop
        // below can move the pointer again.
        //
        // The per-tick snapshot answers exactly that question while it still
        // describes the same target. After a synchronous retarget it belongs
        // to the previous target, so recompute the pair for the replacement
        // instead of dropping the line: Original never leaves `mpMyLineJump`
        // NULL just because the pointer changed, and a dropped line sends the
        // approach at the victim's own sector across the level topology.
        let my_line_jump = if target_snapshot_is_current {
            tick.primary_target_jump_line
        } else {
            // `IsTableSwordfightNeeded` measures with the aggressor's maximal
            // hand-to-hand weapon range (`weapon.distance[Maximal]`), which the
            // fighter snapshot carries as `sword_range_maximal`.
            let my_max_range = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.sword_range_maximal)
                .unwrap_or(self.sword_range);
            grid.and_then(|g| {
                crate::engine::melee::table_swordfight_jump_line(
                    g,
                    ctx.position.sector.map(i16::from).unwrap_or(-1),
                    live_target_pos.sector.map(i16::from).unwrap_or(-1),
                    crate::coordinates::MapPoint::new(live_target_pos.x, live_target_pos.y),
                    my_max_range as f32,
                )
            })
        };
        // `mpMyLineJump = mpMe->IsTableSwordfightNeeded( mpPrimaryTarget )`
        // (`original-code/RHartificialmalignity.cpp:6836`) writes the AI
        // MEMBER, not a local: the answer persists after this decision
        // returns and is read again by `BeginSwordfight`
        // (`:7184`), by the "too far to adversary" gate in
        // `ReconsiderSwordfight` (`:13931`) and — the case that matters
        // here — by `ProposeCombatPositionsAround`'s
        // `bProposePositionsAround = ( mpMyLineJump == NULL )` and its
        // SCOTCHED `mpMyLineJump != NULL` guard (`:14110`, `:14143`).
        // Rust only computed a local, so a fighter standing on a jump
        // line still looked line-less once the fight started and fell
        // through to the 16-direction surround ring the Original never
        // generates.
        self.my_line_jump = my_line_jump;
        let target_animation = if target_snapshot_is_current {
            tick.primary_target_animation
        } else {
            Some(
                ctx.entity_view(self.base.primary_target)
                    .expect("replacement primary target view was resolved above")
                    .current_animation,
            )
        };

        // Target-swap with a same-camp friend if the swap shortens the
        // total travel distance. `friend_swap_candidates` is the
        // engine's enumeration of same-camp soldiers currently
        // approaching an enemy; we walk them in enumeration order and
        // commit the first strict improvement.
        let mut working_target = self.base.primary_target;
        let mut working_target_pos = live_target_pos;
        let mut working_distance = distance;
        let debug_primary_swap = super::primary_swap_debug_enabled()
            && super::primary_swap_debug_matches(ctx.frame, self.base.me);
        // Iterate friends only when we have our own target —
        // Position(primary_target) would crash on NULL otherwise. Skip
        // the swap heuristic if our primary_target is unset so we never
        // hand 0 to a friend via `friend_primary_target_swaps`.
        for cand in &tick.friend_swap_candidates {
            if working_target.is_none() {
                if debug_primary_swap {
                    eprintln!(
                        "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_stop_zero friend={:?}]",
                        ctx.frame, ctx.original_creation_order, self.base.me, cand.friend_id,
                    );
                }
                break;
            }
            if cand.friend_primary_target == working_target {
                if debug_primary_swap {
                    eprintln!(
                        "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_skip_same friend={:?} owner_target={:?} friend_target={:?}]",
                        ctx.frame,
                        ctx.original_creation_order,
                        self.base.me,
                        cand.friend_id,
                        working_target,
                        cand.friend_primary_target,
                    );
                }
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
            let left = me_to_friend_target + friend_to_my_target;
            let right = working_distance + friend_to_friend_target;
            let swap = left < right;
            if debug_primary_swap {
                eprintln!(
                    "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_test friend={:?} owner_target={:?} friend_target={:?} owner_pos=({:08x},{:08x}) owner_target_pos=({:08x},{:08x}) friend_pos=({:08x},{:08x}) friend_target_pos=({:08x},{:08x}) working_distance={:08x} me_to_friend_target={:08x} friend_to_my_target={:08x} friend_to_friend_target={:08x} left={:08x} right={:08x} swap={}]",
                    ctx.frame,
                    ctx.original_creation_order,
                    self.base.me,
                    cand.friend_id,
                    working_target,
                    cand.friend_primary_target,
                    ctx.position.x.to_bits(),
                    ctx.position.y.to_bits(),
                    working_target_pos.x.to_bits(),
                    working_target_pos.y.to_bits(),
                    cand.friend_position.x.to_bits(),
                    cand.friend_position.y.to_bits(),
                    cand.friend_primary_target_position.x.to_bits(),
                    cand.friend_primary_target_position.y.to_bits(),
                    working_distance.to_bits(),
                    me_to_friend_target.to_bits(),
                    friend_to_my_target.to_bits(),
                    friend_to_friend_target.to_bits(),
                    left.to_bits(),
                    right.to_bits(),
                    swap,
                );
            }
            if swap {
                // Each improving friend is retargeted immediately: the
                // reference writes the friend's new primary target on the
                // spot, so several friends can be swapped in a single
                // reconsider pass. Each friend is visited once, so the
                // handed-off target is always the pre-swap working target.
                self.base.outbox.actor.friend_primary_target_swaps.push((
                    cand.friend_id,
                    working_target.expect("friend swap requires current primary target"),
                ));
                working_target = cand.friend_primary_target;
                working_target_pos = cand.friend_primary_target_position;
                working_distance =
                    reconsider_approach_distance(ctx.position, cand.friend_primary_target_position);
                self.base.primary_target = working_target;
            }
        }
        if debug_primary_swap {
            eprintln!(
                "[PRIMARY_SWAP frame={} co={:?} owner={} phase=swap_final target={:?} target_pos=({:08x},{:08x}) distance={:08x} queued_swaps={:?}]",
                ctx.frame,
                ctx.original_creation_order,
                self.base.me,
                working_target,
                working_target_pos.x.to_bits(),
                working_target_pos.y.to_bits(),
                working_distance.to_bits(),
                self.base.outbox.actor.friend_primary_target_swaps,
            );
        }

        // Original tests the `posPrimTarget` that remains after every
        // synchronous target substitution and friend swap. Resolve lift
        // metadata lazily from that final AI `Position(...)`; tick lift data
        // belongs only to the target snapshotted before this owner callback.
        let final_target_lift = AiContext::enemy_lift_approach_for_position(
            &ctx.fast_grid,
            working_target_pos,
            tick.owner_live_position.map(|position| position.level),
        );
        let target_in_lift = final_target_lift.is_some();

        // Primary target is in a non-stairs lift: run to the entry
        // point matching the evaluating NPC's layer.
        if let Some(Some(entry)) = final_target_lift {
            self.base.outbox.actor.set_focus(working_target);
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
                if my_line_jump.is_none() && !target_in_lift {
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
                c &= !target_in_lift;
                tracing::trace!(
                    target: "robin_engine::ai_enemy::charge",
                    frame = ctx.frame,
                    me = self.base.me,
                    charge_weapon = self.sword_is_charge_weapon,
                    courage = self.get_courage(),
                    working_distance,
                    has_line_jump = my_line_jump.is_some(),
                    is_rider = ctx.self_is_rider,
                    target_in_lift,
                    charge = c,
                    "ConsiderCharge: reaction-time charge decision"
                );
                if c {
                    self.base.say(crate::ai::Remark::Warcry);
                }
                (c, true)
            }
            _ => (false, true),
        };

        // Lock eye-tracking onto the primary target.
        self.base.outbox.actor.set_focus(working_target);

        // Riders try charge attack first.
        if ctx.self_is_rider && self.maybe_make_rider_attack(ctx, tick, grid) {
            return;
        }

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=reconsider_close_enough working_distance_bits={:08x} working_distance={} sword_range={} run_distance={} b_charge={} b_first={} my_line_jump={:?} target_in_lift={} working_target={:?}",
                ctx.frame,
                self.base.me,
                working_distance.to_bits(),
                working_distance,
                sword_range,
                run_distance,
                b_charge,
                b_first_consideration,
                my_line_jump,
                target_in_lift,
                working_target,
            );
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
            target_animation,
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
        self.base.outbox.actor.set_focus(working_target);

        // Only a StateChange appended by the approach selected below may own
        // this GoNear prefix. An older matching notification can legitimately
        // still be queued by a recursive caller.
        let owner_work_before_approach = self.base.outbox.reentrant.owner_work.len();
        let mut same_substate_route_split = false;

        // Original GoNear has finished constructing (or rejecting) its route
        // before the following SetState call starts.  A changed-substate
        // SetState captures that prefix in its StateChange notification.  A
        // same-substate SetState has no notification, so split the prefix here
        // instead; otherwise its later SetAttentiveMode tail gets batched in
        // front of the movement by Rust's field-oriented actor drain.
        let split_same_substate_route_before_set_state =
            |this: &mut Self, incoming_substate: Substate| {
                if this.base.current_state != AiState::Attacking
                    || this.base.current_substate != incoming_substate
                {
                    return false;
                }
                assert!(
                    this.base.outbox.actor.has_boundary_work(),
                    "same-substate reconsider approach lost its GoNear prefix"
                );
                this.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                        &mut this.base.outbox.actor,
                    )));
                true
            };

        // Not below run distance: charge or run.
        if !b_below_run_distance {
            if b_charge {
                // GoNear(target, sword_range, RUN | CHARGE).
                self.base.go_near(
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
                same_substate_route_split |= split_same_substate_route_before_set_state(
                    self,
                    Substate::AttackingChargingEnemy,
                );
                self.set_state(AiState::Attacking, Substate::AttackingChargingEnemy);
                self.base.launch_timer(10, ctx.frame);
            } else {
                // GoNear(target, run_distance, RUN | DONT_STOP).
                self.base.go_near(
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
                same_substate_route_split |= split_same_substate_route_before_set_state(
                    self,
                    Substate::AttackingRunningToEnemy,
                );
                self.set_state(AiState::Attacking, Substate::AttackingRunningToEnemy);
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
                    self.base.go_near(
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
                same_substate_route_split |= split_same_substate_route_before_set_state(
                    self,
                    Substate::AttackingRunningToEnemy,
                );
                self.set_state(AiState::Attacking, Substate::AttackingRunningToEnemy);
                self.base.launch_timer(10, ctx.frame);
            } else {
                if my_line_jump.is_none() {
                    // GoNear(target, sword_range, 0) — walk.
                    self.base.go_near(
                        pos_prim_target,
                        standard_sword_range as i32,
                        GotoFlags::empty(),
                        ctx,
                    );
                } else {
                    // GoTo(target) — walk on jump line.
                    self.base.go_to(pos_prim_target, GotoFlags::empty(), ctx);
                }
                if self.base.already_on_point {
                    self.base.already_on_point = false;
                    self.begin_swordfight(ctx, tick);
                    return;
                }
                same_substate_route_split |= split_same_substate_route_before_set_state(
                    self,
                    Substate::AttackingWalkingToEnemy,
                );
                self.set_state(AiState::Attacking, Substate::AttackingWalkingToEnemy);
                self.base.launch_timer(10, ctx.frame);
            }
        }

        // Original path construction is synchronous, so the
        // `mbCouldntReachpoint` test immediately below GoNear observes this
        // attempt's result. Rust settles the queued route after releasing the
        // AI borrow. Resume that exact statement only after the engine has
        // constructed the route; do not turn its failure into an independent
        // unexpected event first.
        let avenger_wait_position = tick.avenger_wait_position_for(self.base.primary_target);
        if self.base.couldnt_reachpoint {
            self.resume_reconsider_enemy_approach_after_go_near(
                working_target_pos,
                avenger_wait_position,
                ctx,
            );
        } else {
            // Original GoNear constructs its route before control reaches the
            // following SetState and the `mbCouldntReachpoint` test
            // (RHartificialmalignity.cpp:6888-6910). SetState captures that
            // actor prefix for its callback barrier; lift it into an explicit
            // earlier owner boundary so route construction is not deferred
            // with ordinary sequence instruction.
            let state_change_index = if same_substate_route_split {
                None
            } else {
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .iter()
                    .enumerate()
                    .skip(owner_work_before_approach)
                    .rev()
                    .find_map(|(index, work)| {
                        matches!(
                            work,
                            crate::ai::AiOwnerWork::StateChange(notification)
                                if notification.incoming_state == self.base.current_state
                                    && notification.incoming_substate
                                        == self.base.current_substate
                        )
                        .then_some(index)
                    })
            };
            if let Some(state_change_index) = state_change_index {
                let route_effects =
                    match &mut self.base.outbox.reentrant.owner_work[state_change_index] {
                        crate::ai::AiOwnerWork::StateChange(notification) => notification
                            .actor_effects_before_callback
                            .take()
                            .expect("reconsider approach GoNear was not captured before SetState"),
                        _ => unreachable!(),
                    };
                self.base.outbox.reentrant.owner_work.insert(
                    state_change_index,
                    crate::ai::AiOwnerWork::ActorEffects(route_effects),
                );
            } else if !same_substate_route_split {
                // SetState deliberately omits FilterAIEvent when the selected
                // substate is unchanged. GoNear therefore remains in the live
                // actor outbox, but Original still constructs it synchronously
                // before testing mbCouldntReachpoint.
                assert!(
                    self.base.outbox.actor.has_boundary_work(),
                    "reconsider approach same-substate GoNear lost its actor effects"
                );
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                        &mut self.base.outbox.actor,
                    )));
            }
            // SetState's virtual attentive-mode tail also precedes the return
            // to `mbCouldntReachpoint`.
            if self.base.outbox.actor.has_boundary_work() {
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                        &mut self.base.outbox.actor,
                    )));
            }
            self.base
                .outbox
                .reentrant
                .reconsider_approach_completion_pending = true;
            self.base.outbox.reentrant.owner_work.push(
                crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear {
                    target: self
                        .base
                        .primary_target
                        .expect("deferred enemy approach requires a target")
                        .get(),
                    target_position: working_target_pos,
                },
            );
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=reconsider_deferred state={:?}/{:?} primary={:?} target_position=({:08x},{:08x},sector={:?},level={}) couldnt={} already={} owner_work={:?}",
                    ctx.frame,
                    self.base.me,
                    self.base.current_state,
                    self.base.current_substate,
                    self.base.primary_target,
                    working_target_pos.x.to_bits(),
                    working_target_pos.y.to_bits(),
                    working_target_pos.sector,
                    working_target_pos.level,
                    self.base.couldnt_reachpoint,
                    self.base.already_on_point,
                    self.base.outbox.reentrant.owner_work,
                );
            }
        }
    }

    pub(crate) fn resume_reconsider_enemy_approach_after_go_near(
        &mut self,
        target_position: Position,
        avenger_wait_position: Option<Position>,
        ctx: &AiContext,
    ) {
        let halt_roof_fallback_after_launch = std::mem::take(
            &mut self
                .base
                .outbox
                .reentrant
                .reconsider_approach_replaced_path_waiter,
        );
        let debug_decision_path = super::decision_path_debug_enabled()
            && super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} co={:?} stage=reconsider_resume_enter state={:?}/{:?} couldnt={} already={} target_position=({:08x},{:08x},sector={:?},level={}) avenger_wait={:?} owner_work={:?}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                self.base.current_state,
                self.base.current_substate,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                target_position.x.to_bits(),
                target_position.y.to_bits(),
                target_position.sector,
                target_position.level,
                avenger_wait_position,
                self.base.outbox.reentrant.owner_work,
            );
        }
        if !self.base.couldnt_reachpoint {
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=reconsider_resume_result result=route_ok state={:?}/{:?}",
                    ctx.frame, self.base.me, self.base.current_state, self.base.current_substate,
                );
            }
            return;
        }
        let Some(wait_pos) = avenger_wait_position else {
            // Original returns without clearing mbCouldntReachpoint when the
            // reverse gate walk cannot find a blocking gate.
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=reconsider_resume_result result=failed_without_wait_position couldnt=true",
                    ctx.frame, self.base.me,
                );
            }
            return;
        };

        self.base.couldnt_reachpoint = false;
        self.set_state(AiState::Attacking, Substate::AttackingRunToAvengerOnRoof);
        let pending_orders_before = self.base.outbox.actor.orders.len();
        self.base.go_near(wait_pos, 50, GotoFlags::RUN, ctx);
        // When the failed approach replaced a live MoveWaiting, Original still
        // observes that command in this recursive GoTo's final
        // `IsComputingPath()` check. It launches the roof sequence and then
        // Halt removes it from the manager FIFO before instruction. A failure
        // constructed from an ordinary Wait skips that tail Halt and the roof
        // sequence starts this frame.
        // GoNear can succeed synchronously without launching a sequence when
        // the actor is already within the 50-unit tolerance. Original leaves
        // mbAlreadyOnPoint set for the enclosing EndThink in that case, then
        // recursively enters EVENT_REACHPOINT in the roof-run substate. Only
        // an actually launched replacement sequence can inherit the
        // path-waiter's trailing Halt.
        if self.base.outbox.actor.orders.len() > pending_orders_before
            && let Some(order) = self.base.outbox.actor.orders.last_mut()
        {
            order.halt_after_launch_for_path_waiter = halt_roof_fallback_after_launch;
        }
        // The wait position is only the reachable staging point. Original
        // keeps the actual avenger position for the later face/wait behavior.
        self.base.seek_position = target_position;
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=reconsider_resume_result result=avenger_fallback state={:?}/{:?} couldnt={} already={} owner_work={:?}",
                ctx.frame,
                self.base.me,
                self.base.current_state,
                self.base.current_substate,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
            );
        }
    }

    /// Resume Original's `DECISION_OBSERVE` statement immediately after its
    /// first synchronous `GoNear`. A successful/no-roof path registers the
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
        self.set_state(AiState::Attacking, Substate::AttackingApproachToObserve);
        self.base.launch_timer(50, ctx.frame);

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
    fn compute_jump_line_target(
        &self,
        grid: &crate::fast_find_grid::FastFindGrid,
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
    pub fn maybe_make_rider_attack(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        let debug_decision_path = super::decision_path_debug_enabled()
            && super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} co={:?} stage=rider_attack_enter state={:?}/{:?} primary={:?} rider={} position=({:08x},{:08x},sector={:?},level={}) direction={} list_them={:?} fighters={}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                self.base.current_state,
                self.base.current_substate,
                self.base.primary_target,
                ctx.self_is_rider,
                ctx.position.x.to_bits(),
                ctx.position.y.to_bits(),
                ctx.position.sector,
                ctx.position.level,
                ctx.direction,
                self.list_them,
                tick.nearby_fighters.len(),
            );
        }
        assert!(ctx.self_is_rider);

        let my_pos = ctx.position;
        let my_dir = ctx.direction;

        // Try primary target first.
        let mut target = self.base.primary_target;
        let mut dest = Position::default();
        let mut begin_charge = false;
        let mut ok = false;

        // Find the primary target from fighter snapshots. Original
        // `GetGoodRiderAttackDestination` reads `pEnemy->GetPositionMap()`,
        // not the door/carrier-aware AI `Position(pEnemy)`, so charge
        // geometry consumes the raw element position. The persistent target
        // pointer is not radius-limited, so fall back to the full fighter
        // registry when it is outside the 500-unit `nearby_fighters` window.
        let target_snapshot = self.find_fighter(target, tick).filter(|f| !f.is_friendly);

        if target.is_some()
            && let Some(target_snapshot) = target_snapshot
        {
            // The reference checks !IsDead && !IsUnconscious && !IsTied.
            // `is_able_to_fight` already covers the first two; the
            // explicit `is_tied` check separates the bound posture
            // (where the engine still has the entity active so
            // `is_able_to_fight` could return true).
            let target_alive = self
                .find_fighter(target, tick)
                .map(|f| f.is_able_to_fight && !f.is_tied)
                .unwrap_or(false);

            if target_alive
                && let Some((d, bc)) = self.get_good_rider_attack_destination(
                    target.expect("rider target presence was checked").get(),
                    my_pos,
                    my_dir,
                    target_snapshot.raw_position,
                    ctx,
                    grid,
                    &tick.fighter_registry,
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
                if Some(AiEntityHandle::new(*enemy)) == target {
                    continue;
                }
                // Original's fallback scan walks `mlistThem` and calls
                // GetGoodRiderAttackDestination on every non-primary entry
                // with no liveness or radius prefilter — the list is already
                // maintained as the fight-capable enemy set. Resolve the
                // entry through the full fighter registry so enemies beyond
                // the 500-unit `nearby_fighters` window still get evaluated,
                // and preserve that function's direct GetPositionMap read.
                let epos = match self.find_fighter(*enemy, tick).map(|f| f.raw_position) {
                    Some(p) => p,
                    None => continue,
                };
                if let Some((d, bc)) = self.get_good_rider_attack_destination(
                    *enemy,
                    my_pos,
                    my_dir,
                    epos,
                    ctx,
                    grid,
                    &tick.fighter_registry,
                ) {
                    target = Some(AiEntityHandle::new(*enemy));
                    self.base.primary_target = target;
                    dest = d;
                    begin_charge = bc;
                    ok = true;
                    break;
                }
            }
        }

        tracing::trace!(
            target: "robin_engine::ai_enemy::charge",
            frame = ctx.frame,
            me = self.base.me,
            ok,
            ?target,
            begin_charge,
            "RiderCharge: destination search"
        );
        if !ok {
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=rider_attack_result result=no_destination final_primary={:?} couldnt={} already={} owner_work={:?}",
                    ctx.frame,
                    self.base.me,
                    self.base.primary_target,
                    self.base.couldnt_reachpoint,
                    self.base.already_on_point,
                    self.base.outbox.reentrant.owner_work,
                );
            }
            return false;
        }

        // Original `MaybeMakeRiderAttack` unconditionally dereferences the
        // selected `mpPrimaryTarget` here.  The target search above already
        // uses the full fighter registry because the pointer is not limited
        // to the 500-unit nearby snapshot; preserve that same scope when
        // publishing `mposSeekPosition` for the later rider-return Face.
        self.base.seek_position = self
            .find_fighter(target, tick)
            .unwrap_or_else(|| {
                panic!(
                    "selected rider charge target {target:?} disappeared from the fighter registry"
                )
            })
            .position;

        // Original focuses the selected target before choosing between the
        // approach and immediate-charge arms. The immediate-charge arm then
        // deliberately replaces this with Focus(NULL), while the approach
        // keeps EYES_FOLLOW. Besides steering the gaze, Follow bypasses the
        // Lacklandist optical cadence so visibility refreshes immediately.
        self.base.outbox.actor.set_focus(target);

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
            self.base.outbox.actor.set_unfocus();
            self.base.say(crate::ai::Remark::Warcry);
            self.go_to(
                AiState::Attacking,
                Substate::AttackingRiderChargingPassing,
                dest,
                GotoFlags::RUN | GotoFlags::RIDER_CHARGE | GotoFlags::RIDER_CHARGE_HIT,
                ctx,
            );
        }

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=rider_attack_result result=accepted target={} destination=({:08x},{:08x},sector={:?},level={}) begin_charge={} state={:?}/{:?} couldnt={} already={} owner_work={:?}",
                ctx.frame,
                self.base.me,
                target,
                dest.x.to_bits(),
                dest.y.to_bits(),
                dest.sector,
                dest.level,
                begin_charge,
                self.base.current_state,
                self.base.current_substate,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
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
        candidate: HumanHandle,
        my_pos: Position,
        my_dir: u16,
        enemy_pos: Position,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        fighter_registry: &[FighterSnapshot],
    ) -> Option<(Position, bool)> {
        let debug_decision_path = super::decision_path_debug_enabled()
            && super::decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=rider_candidate_enter candidate={} me=({:08x},{:08x},level={}) enemy=({:08x},{:08x},level={}) direction={} move_box={:?}",
                ctx.frame,
                self.base.me,
                candidate,
                my_pos.x.to_bits(),
                my_pos.y.to_bits(),
                my_pos.level,
                enemy_pos.x.to_bits(),
                enemy_pos.y.to_bits(),
                enemy_pos.level,
                my_dir,
                ctx.move_box,
            );
        }
        let geometry = match rider_charge_goal_geometry(
            (my_pos.x, my_pos.y),
            my_dir,
            (enemy_pos.x, enemy_pos.y),
        ) {
            Ok(geometry) => geometry,
            Err(reject) => {
                if debug_decision_path {
                    match reject {
                        RiderChargeReject::Behind { forward_dot } => eprintln!(
                            "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_behind forward_dot_bits={:08x}",
                            ctx.frame,
                            self.base.me,
                            candidate,
                            forward_dot.to_bits(),
                        ),
                        RiderChargeReject::TooNear { norm, sq_norm } => eprintln!(
                            "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_too_near norm_bits={:08x} sq_norm_bits={:08x}",
                            ctx.frame,
                            self.base.me,
                            candidate,
                            norm.to_bits(),
                            sq_norm.to_bits(),
                        ),
                        RiderChargeReject::ZeroOrthogonal { ortho_len } => eprintln!(
                            "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_orthogonal ortho_len_bits={:08x}",
                            ctx.frame,
                            self.base.me,
                            candidate,
                            ortho_len.to_bits(),
                        ),
                        RiderChargeReject::ZeroHitVector { hp_len } => eprintln!(
                            "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_hit_vector hp_len_bits={:08x}",
                            ctx.frame,
                            self.base.me,
                            candidate,
                            hp_len.to_bits(),
                        ),
                        RiderChargeReject::ZeroHitNorm { hit_norm_len } => eprintln!(
                            "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_zero_hit_norm hit_norm_bits={:08x}",
                            ctx.frame,
                            self.base.me,
                            candidate,
                            hit_norm_len.to_bits(),
                        ),
                    }
                }
                return None;
            }
        };
        let RiderChargeGeometry {
            forward_dot,
            sq_norm,
            cos_alpha,
            me_to_hit,
            hit_dir,
            hit_norm_len,
            goal: (goal_x, goal_y),
        } = geometry;

        // Check if straight movement from me to goal is clear.
        if let Some(g) = grid {
            let pt_me = crate::coordinates::MapPoint::new(my_pos.x, my_pos.y);
            let pt_goal = crate::coordinates::MapPoint::new(goal_x, goal_y);
            if !g.is_straight_movement_authorized(pt_me, pt_goal, my_pos.level, &ctx.move_box) {
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_straight goal=({:08x},{:08x}) forward_dot_bits={:08x} sq_norm_bits={:08x} cos_bits={:08x}",
                        ctx.frame,
                        self.base.me,
                        candidate,
                        goal_x.to_bits(),
                        goal_y.to_bits(),
                        forward_dot.to_bits(),
                        sq_norm.to_bits(),
                        cos_alpha.to_bits(),
                    );
                }
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

                let p0 = (first_corner.0, first_corner.1);
                let p1 = (
                    first_corner.0 + dir_norm.0 * loop_d,
                    first_corner.1 + dir_norm.1 * loop_d,
                );
                let p2 = (
                    first_corner.0 + dir_norm.0 * loop_d + normal.0 * lat_d,
                    first_corner.1 + dir_norm.1 * loop_d + normal.1 * lat_d,
                );
                let p3 = (
                    first_corner.0 + normal.0 * lat_d,
                    first_corner.1 + normal.1 * lat_d,
                );

                let poly = geo::Polygon::new(
                    geo::LineString::from(vec![
                        (p0.0 as f64, p0.1 as f64),
                        (p1.0 as f64, p1.1 as f64),
                        (p2.0 as f64, p2.1 as f64),
                        (p3.0 as f64, p3.1 as f64),
                        (p0.0 as f64, p0.1 as f64),
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
                // Original IsAnyFriendInThisPolygon scans the engine's
                // complete same-camp fighter registry. A friend can block the
                // far end of this charge corridor while being outside the
                // rider-centered nearby-fighter window. Its polygon test also
                // reads each friend's raw GetPositionMap, not AI Position().
                for f in fighter_registry {
                    if !f.is_friendly || f.handle == 0 {
                        continue;
                    }
                    if f.handle == self.base.me {
                        continue;
                    }
                    if f.is_dead || f.is_unconscious {
                        continue;
                    }
                    if f.raw_position.level != my_pos.level {
                        continue;
                    }
                    let fp = geo::Point::new(f.raw_position.x as f64, f.raw_position.y as f64);
                    if poly.contains(&fp) {
                        if debug_decision_path {
                            eprintln!(
                                "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=reject_friendly friendly={} friendly_position=({:08x},{:08x},level={}) goal=({:08x},{:08x})",
                                ctx.frame,
                                self.base.me,
                                candidate,
                                f.handle,
                                f.raw_position.x.to_bits(),
                                f.raw_position.y.to_bits(),
                                f.raw_position.level,
                                goal_x.to_bits(),
                                goal_y.to_bits(),
                            );
                        }
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

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=rider_candidate_result candidate={} result=accepted goal=({:08x},{:08x},sector={:?},level={}) forward_dot_bits={:08x} sq_norm_bits={:08x} cos_bits={:08x} sq_hit_bits={:08x} begin_charge={}",
                ctx.frame,
                self.base.me,
                candidate,
                destination.x.to_bits(),
                destination.y.to_bits(),
                destination.sector,
                destination.level,
                forward_dot.to_bits(),
                sq_norm.to_bits(),
                cos_alpha.to_bits(),
                sq_hit_dist.to_bits(),
                begin_charge_anim,
            );
        }

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
        let pt_me = crate::coordinates::MapPoint::new(my_pos.x, my_pos.y);

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
        sim: &crate::sim_rng::SimulationContext,
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
            self.battle_decisions(sim, global, ctx, tick, grid);
        }
    }

    // -----------------------------------------------------------------------
    // BeginSwordfight
    // -----------------------------------------------------------------------

    pub fn begin_swordfight(&mut self, ctx: &AiContext, _tick: &AiPerTickData) {
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

        // Original enters swordfight by calling both reciprocal Update*
        // setters, not by dropping only this soldier's local pointers
        // (RHartificialmalignity.cpp:7191-7192).
        self.clear_combat_neighbours();

        // Release eye-tracking lock on swordfight entry so the soldier's
        // focus arrow / cone stops chasing the previous focus target.
        self.base.outbox.actor.set_unfocus();

        // Ask the engine to apply BeginSwordfight's conditional target Stop
        // at the outbox-drain boundary. The reference reads the target's live
        // action state here, after every earlier-created entity has already
        // run its Hourglass. `tick.fighters` is the pre-entity snapshot and
        // can still say Waiting when the target completed its start-running
        // transition earlier in this same frame. The engine therefore owns
        // both live gates (`!IsSwordfighting` and Moving/MovingFast).
        self.base.outbox.actor.stop_target = Some(
            self.base
                .primary_target
                .expect("swordfight entry target presence was checked"),
        );

        // No SetDirection here. Direction is set by the engine-side
        // ENTER_SWORDFIGHT pipeline: the
        // `RHANIMATION_TRANSITION_RAISING_SWORD` order carries the
        // opponent as `pAntagonist`, and the soldier's per-tick
        // execute handler calls `SetDirection(opponent - me)` on
        // initialisation, then `Turn()` rotates the body each frame.
        // Mirrored at the order-launch sites in
        // `EngineInner::dispatch_enter_swordfight` and
        // `EngineInner::enter_swordfight_with_jump_line`.

        // Tell the engine to call enter_swordfight(me, target) so both
        // entities get added to each other's opponent lists and action
        // states transition to sword combat. Original BeginSwordfight reads
        // the persistent `mpMyLineJump` selected by ReconsiderEnemyApproach;
        // it does not recompute the line from the target's current position.
        // The target may have moved away from the edge during the approach,
        // but the retained line still has to become
        // RHFIELD_JUMPLINE_DESTINATION on ENTER_SWORDFIGHT.
        self.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::Engage(
            self.base
                .primary_target
                .expect("swordfight entry target presence was checked")
                .get(),
        ));
        self.base.outbox.actor.enter_swordfight_jump_line = self.my_line_jump;

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

/// Pure geometry core of `RHArtificialMalignity::GetGoodRiderAttackDestination`
/// (RHartificialmalignity.cpp:19796).
///
/// The charge goal feeds the movement order verbatim, so this math is
/// save-observable to the last bit. Two shapes are easy to get wrong:
///
/// * The nose vector is `SetSector0to15( GetDirection() )` with the
///   **default** aspect ratio `1.0f` (SBGeoVector2D.h:79) — the raw
///   stretched-space table entry. Applying `ASPECT_RATIO` and then
///   unapplying `INVERSE_ASPECT_RATIO` lands an ULP off and can flip the
///   forward half-plane test for boundary vectors.
/// * Both `SBGeoVector2D::operator*=( GEOTYPE k )` sites round their
///   scalar **once** before touching the components (`Set(k*mX, k*mY)`,
///   SBGeoVector2D.cpp:97-102): `k1 = RIDER_CHARGE_LATERAL_DISTANCE /
///   fCosAlpha` (RHartificialmalignity.cpp:19871) and `k2 = fCosAlpha *
///   fMeToEnemyNorm` (:19878). Distributing the multiply per component
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

    // vNoseVectorStretchedY.SetSector0to15( direction ) — default aspect 1.0.
    let nose_sy = sector_to_vector_iso(my_dir, 1.0);

    // Is the enemy before me?
    let forward_dot = dot2(nose_sy, me_to_enemy_sy);
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

    // GetNormal( false ) with aspect 1.0 yields (mY, -mX); then Normalize().
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

    // vHitPointToGoal = GetNormalized() * RIDER_CHARGE_LOOP_DISTANCE;
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
mod tests {
    use super::*;

    #[test]
    fn lost_pc_overview_uses_handle_keyed_forecast_instead_of_stale_seek_position() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(66);
        ai.missed_pc = 126;
        ai.pc_gone_away_in_this_direction = 7;
        ai.base.seek_position = Position {
            x: 1810.0,
            y: 1155.0,
            sector: crate::position_interface::SectorHandle::new(78),
            level: 2,
        };
        let forecast = Position {
            x: 1759.0,
            y: 1033.0,
            sector: crate::position_interface::SectorHandle::new(75),
            level: 4,
        };
        let mut tick = AiPerTickData::stub();
        tick.enemy_detectable_forecasts.push((
            126,
            crate::ai::PreparedForecastDestination::fixed(forecast, 8),
        ));

        ai.refresh_missed_pc_forecast(&sim, &tick);

        assert_eq!(ai.base.seek_position, forecast);
        assert_eq!(ai.pc_gone_away_in_this_direction, 8);
    }

    #[test]
    #[should_panic(expected = "lost-PC overview target 126 has no prepared destination forecast")]
    fn lost_pc_overview_never_falls_back_to_stale_seek_position() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(66);
        ai.missed_pc = 126;
        ai.base.seek_position = Position {
            x: 1810.0,
            y: 1155.0,
            sector: crate::position_interface::SectorHandle::new(78),
            level: 2,
        };

        ai.refresh_missed_pc_forecast(&sim, &AiPerTickData::stub());
    }

    #[test]
    fn begin_swordfight_publishes_reciprocal_combat_neighbour_clears() {
        // RHArtificialMalignity::BeginSwordfight clears the formation with
        // UpdateLeft/RightCombatNeighbour(NULL). Dropping only the local
        // pointers leaves stale back-pointers that a later phalanx insertion
        // can follow and use to detach a live chain.
        let mut ai = EnemyAi::new(132);
        ai.base.primary_target = 343;
        ai.left_combat_neighbour = 131;
        ai.right_combat_neighbour = 133;

        ai.begin_swordfight(&AiContext::default(), &AiPerTickData::stub());

        assert_eq!(ai.left_combat_neighbour, 0);
        assert_eq!(ai.right_combat_neighbour, 0);
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [
                CrossNpcAction::SetRightCombatNeighbour {
                    target: 131,
                    neighbour: 0,
                },
                CrossNpcAction::SetLeftCombatNeighbour {
                    target: 133,
                    neighbour: 0,
                },
            ]
        ));
        assert!(
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic))
        );
        assert!(
            !ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic180))
        );
    }

    #[test]
    fn begin_swordfight_retains_approach_jump_line_when_live_probe_is_none() {
        // RHArtificialMalignity::ReconsiderEnemyApproach stores the chosen
        // line in mpMyLineJump. BeginSwordfight later copies that member to
        // RHFIELD_JUMPLINE_DESTINATION even if the victim has moved far enough
        // that a fresh IsTableSwordfightNeeded probe would now return null.
        let mut ai = EnemyAi::new(222);
        ai.base.primary_target = 317;
        ai.my_line_jump = Some(19);
        let mut tick = AiPerTickData::stub();
        tick.primary_target_jump_line = None;

        ai.begin_swordfight(&AiContext::default(), &tick);

        assert_eq!(ai.my_line_jump, Some(19));
        let staged = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                AiOwnerWork::StateChange(notification) => {
                    notification.actor_effects_before_callback.as_ref()
                }
                _ => None,
            })
            .expect("BeginSwordfight actor effects must precede its SetState callback");
        assert_eq!(staged.enter_swordfight_jump_line, Some(19));
    }

    /// nicouzouf Savegame_047 replay-004, frame 563: Soldier51 (a rider in
    /// AttackingReactiontimeRunning) plans a charge approach against Pc76.
    /// Inputs captured bit-exact from the parity replay. The Original's
    /// `operator*=` sites round `RIDER_CHARGE_LATERAL_DISTANCE / fCosAlpha`
    /// and `fCosAlpha * fMeToEnemyNorm` once before the component
    /// multiplies; the per-component `n * 40.0 / cos` order previously
    /// produced goal.y = 0x4425e9c8 (one ULP low), which propagated
    /// through the stop-transition splice into the running order's goal,
    /// its normalized increment, and the frame-564 movement_map drift.
    #[test]
    fn rider_charge_goal_matches_original_scalar_rounding() {
        let me = (f32::from_bits(0x448f_3c66), f32::from_bits(0x43dc_a7ea));
        let enemy = (f32::from_bits(0x443a_7ea7), f32::from_bits(0x4418_a6d2));

        let geometry = match rider_charge_goal_geometry(me, 11, enemy) {
            Ok(geometry) => geometry,
            Err(_) => panic!("frame-563 fixture must produce a charge goal"),
        };

        assert_eq!(geometry.goal.0.to_bits(), 0x442f_2b23);
        assert_eq!(geometry.goal.1.to_bits(), 0x4425_e9c9);
        // The strike-zone / begin-charge inputs the caller consumes.
        assert_eq!(geometry.me_to_hit.0.to_bits(), 0xc3ba_d233);
        assert_eq!(geometry.hit_norm_len.to_bits(), 0x43d0_d28f);
    }

    #[test]
    fn rider_charge_friend_corridor_uses_full_fighter_registry() {
        // nicouzouf Profile_001 Savegame_047 replay-005, frame 1433:
        // Soldier62 is outside Soldier51's nearby-fighter window but stands
        // inside the strike corridor toward PC76. Original's global
        // GetNumberOfFighters/GetFighter scan rejects this charge.
        let mut ai = EnemyAi::new(51);
        ai.base.primary_target = 76;
        ai.list_them = vec![76];
        let rider = Position {
            x: f32::from_bits(0x44c7_1d8e),
            y: f32::from_bits(0x4421_e39e),
            ..Position::default()
        };
        let target = FighterSnapshot {
            handle: 76,
            position: Position {
                x: f32::from_bits(0x4474_03e3),
                y: f32::from_bits(0x43bb_a89f),
                ..Position::default()
            },
            raw_position: Position {
                x: f32::from_bits(0x4474_03e3),
                y: f32::from_bits(0x43bb_a89f),
                ..Position::default()
            },
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        };
        let blocking_friend = FighterSnapshot {
            handle: 62,
            position: Position {
                x: f32::from_bits(0x447b_182c),
                y: f32::from_bits(0x43b8_eb5e),
                ..Position::default()
            },
            raw_position: Position {
                x: f32::from_bits(0x447b_182c),
                y: f32::from_bits(0x43b8_eb5e),
                ..Position::default()
            },
            is_friendly: true,
            is_soldier: true,
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![target.clone()];
        tick.fighter_registry = vec![target, blocking_friend];
        let ctx = AiContext {
            self_is_rider: true,
            position: rider,
            direction: 11,
            ..AiContext::default()
        };

        assert!(!ai.maybe_make_rider_attack(&ctx, &tick, None));
        assert_eq!(ai.base.primary_target, 76);
        assert!(ai.base.outbox.actor.orders.is_empty());
    }

    #[test]
    fn rider_charge_retains_out_of_range_target_position_for_return_face() {
        // Original `MaybeMakeRiderAttack` stores Position(mpPrimaryTarget)
        // after selecting the charge.  A raw target pointer remains valid
        // outside Rust's radius-limited nearby-fighter snapshot; the later
        // GettingDistance reach-point handler faces this stored position.
        let mut ai = EnemyAi::new(51);
        ai.base.primary_target = 76;
        ai.base.seek_position = Position {
            x: 900.0,
            y: 700.0,
            ..Position::default()
        };
        let target_position = Position {
            x: 0.0,
            y: -200.0,
            ..Position::default()
        };
        let target = FighterSnapshot {
            handle: 76,
            position: target_position,
            raw_position: target_position,
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters.clear();
        tick.fighter_registry = vec![target];
        let ctx = AiContext {
            self_is_rider: true,
            position: Position::default(),
            direction: 0,
            ..AiContext::default()
        };

        assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));
        assert_eq!(ai.base.primary_target, 76);
        assert_eq!(ai.base.seek_position, target_position);
    }

    #[test]
    fn rider_charge_geometry_uses_raw_target_position_during_door_transit() {
        // `Position(target)` substitutes the active door endpoint, while
        // GetGoodRiderAttackDestination reads target->GetPositionMap(). Put
        // those points on opposite sides of the rider so the accessor choice
        // is observable without reproducing the shipped door grid.
        let mut ai = EnemyAi::new(51);
        ai.base.primary_target = 76;
        ai.list_them = vec![76];
        let target = FighterSnapshot {
            handle: 76,
            position: Position {
                y: -50.0,
                level: 1,
                ..Position::default()
            },
            raw_position: Position {
                y: 50.0,
                ..Position::default()
            },
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![target.clone()];
        tick.fighter_registry = vec![target];
        let ctx = AiContext {
            self_is_rider: true,
            direction: 0,
            ..AiContext::default()
        };
        let initial_state = (ai.base.current_state, ai.base.current_substate);

        assert!(!ai.maybe_make_rider_attack(&ctx, &tick, None));
        assert_eq!(
            (ai.base.current_state, ai.base.current_substate),
            initial_state
        );
        assert!(ai.base.outbox.actor.orders.is_empty());
    }

    fn pc_view() -> crate::ai_entity_view::AiEntityView {
        let entity = crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: crate::element::PcData {
                life_points: 100,
                ..Default::default()
            },
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

    fn pc_view_at(position: Position) -> crate::ai_entity_view::AiEntityView {
        let mut view = pc_view();
        view.position = position;
        view.detection_position = crate::coordinates::MapPoint::new(position.x, position.y);
        view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(position.x, position.y, 0.0);
        view
    }

    fn add_owner_sword_range(tick: &mut AiPerTickData, owner: u32, range: u16) {
        tick.fighter_registry.push(FighterSnapshot {
            handle: owner,
            sword_range_default: range,
            ..FighterSnapshot::default()
        });
    }

    #[test]
    fn attack_enemy_prefers_matching_position_snapshot_over_fighter_geometry() {
        // RHArtificialMalignity::AttackEnemy writes
        // `mposSeekPosition = Position(pEnemy)`. The target-specific tick
        // field is that source read; nearby-fighter geometry can represent an
        // older owner boundary and must not replace it for the same handle.
        let mut ai = EnemyAi::new(104);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;

        let authoritative = Position {
            x: 1578.1302,
            y: 1894.2336,
            sector: crate::position_interface::SectorHandle::new(119),
            level: 8,
        };
        let stale_fighter_position = Position {
            x: 1524.8396,
            y: 1704.1648,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };
        let mut tick = AiPerTickData::stub();
        tick.primary_target_snapshot_handle = 252;
        tick.primary_target_position = Some(authoritative);
        tick.nearby_fighters.push(FighterSnapshot {
            handle: 252,
            position: stale_fighter_position,
            ..FighterSnapshot::default()
        });
        // Force ReconsiderEnemyApproach's already-fighting early return so
        // the assertion observes AttackEnemy's assignment directly.
        let ctx = AiContext {
            is_swordfighting: true,
            ..AiContext::default()
        };

        ai.attack_enemy(252, None, &ctx, &tick, None);

        assert_eq!(ai.base.primary_target, 252);
        assert_eq!(ai.base.seek_position, authoritative);
    }

    #[test]
    fn attack_enemy_retarget_uses_live_exact_sector_over_number_only_fighter_snapshot() {
        // Continue/replay-026, frame 7896: BattleDecisions initially
        // snapshots PC282, then an attacking friend contributes PC137. Both
        // target positions use public sector 88, but only the live Position()
        // carries the arena object needed to find gates 111 and 114.
        let mut ai = EnemyAi::new(153);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontimeRunning;

        let public = crate::position_interface::SectorHandle::new(88).unwrap();
        let exact = public.with_arena_index(crate::fast_find_grid::SectorIndex::new(137).unwrap());
        let live_target = Position {
            x: 684.1841,
            y: 1545.0576,
            sector: Some(exact),
            level: 2,
        };
        let number_only = Position {
            sector: Some(public),
            ..live_target
        };

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(137, pc_view_at(live_target));
        let ctx = AiContext {
            is_swordfighting: true,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.primary_target_snapshot_handle = 282;
        tick.primary_target_position = Some(Position {
            x: 900.0,
            y: 1800.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        });
        tick.nearby_fighters.push(FighterSnapshot {
            handle: 137,
            position: number_only,
            ..FighterSnapshot::default()
        });

        ai.attack_enemy(137, None, &ctx, &tick, None);

        assert_eq!(ai.base.primary_target, 137);
        assert_eq!(ai.base.seek_position, live_target);
        assert_eq!(
            ai.base.seek_position.sector.unwrap().arena_index(),
            Some(crate::fast_find_grid::SectorIndex::new(137).unwrap())
        );
    }

    fn reconsider_approach_lift_grid() -> crate::fast_find_grid::FastFindGrid {
        let mut grid = crate::fast_find_grid::FastFindGrid::new();
        let lift_number = crate::sector::SectorNumber::new(42);
        let ordinary_number = crate::sector::SectorNumber::new(5);
        let level = std::sync::Arc::make_mut(&mut grid.level);
        level.sector_number_map.insert(lift_number, 0);
        level.sector_number_map.insert(ordinary_number, 1);
        level.door_projection_infos = vec![
            crate::fast_find_grid::DoorProjectionInfo {
                point_out: crate::coordinates::MapPoint::new(410.0, 120.0),
                sector_out: crate::sector::SectorNumber::new(7),
                layer_out: 3,
                ..Default::default()
            },
            crate::fast_find_grid::DoorProjectionInfo {
                point_out: crate::coordinates::MapPoint::new(430.0, 300.0),
                sector_out: ordinary_number,
                layer_out: 0,
                ..Default::default()
            },
        ];
        let sector = |sector_number, sector_type, lift_type, gate_indices| {
            crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type,
                layer: 0,
                sector_number,
                door_index: None,
                lift_type,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices,
                underlying_sector: None,
            }
        };
        level.sectors.push(sector(
            lift_number,
            crate::sector::SectorType::LIFT,
            Some(crate::sector::LiftType::Ladder),
            vec![
                crate::gate::DoorIndex::new(0).expect("valid door index"),
                crate::gate::DoorIndex::new(1).expect("valid door index"),
            ],
        ));
        level.sectors.push(sector(
            ordinary_number,
            crate::sector::SectorType::AREA | crate::sector::SectorType::MOTION,
            None,
            Vec::new(),
        ));
        grid
    }

    #[test]
    fn failed_look_for_help_route_is_consumed_before_event_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(105);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingRunningToOfficer;
        ai.base.couldnt_reachpoint = true;
        ai.base.primary_target = 252;
        ai.list_them = vec![252];

        let threat = Position {
            x: 1050.0,
            y: 1780.0,
            ..Position::default()
        };
        let stale_threat = Position {
            x: 2145.0,
            y: 1976.0,
            ..Position::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.fighter_registry = vec![FighterSnapshot {
            handle: 252,
            position: stale_threat,
            is_pc: true,
            is_able_to_fight: true,
            ..FighterSnapshot::default()
        }];
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        let target_view = pc_view_at(threat);
        views.insert(252, target_view);
        views.insert(105, pc_view_at(Position::default()));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut global = AiGlobalState::default();

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            ai.resume_battle_look_for_help_after_alert_officer(&sim, &mut global, &ctx, &tick);
        });

        assert_eq!(draws, vec![crate::sim_rng::RngSite::BattlePanicRemark]);
        assert!(!ai.base.couldnt_reachpoint);
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert!(ai.my_seek_points.is_empty());
        let panic = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("failed LookForHelp must continue through Cassos Panic");
        assert_eq!(panic.center, Some(threat));
        let log = ai.base.ai_log.last().expect("Cassos decision log");
        assert_eq!(log.line_type, LogLineType::BattleDecision);
        assert_eq!(log.info, Decision::Cassos as u16);
    }

    #[test]
    fn cassos_uses_live_target_position_instead_of_stale_fighter_or_seek_position() {
        let mut ai = EnemyAi::new(117);
        ai.base.seek_position = Position {
            x: 2145.0,
            y: 1976.0,
            ..Position::default()
        };
        let live = Position {
            x: 873.0,
            y: 1717.0,
            sector: crate::position_interface::SectorHandle::new(309),
            level: 8,
        };
        let stale = Position {
            x: 900.0,
            y: 1800.0,
            ..Position::default()
        };
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(252, pc_view_at(live));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters.push(FighterSnapshot {
            handle: 252,
            position: stale,
            ..FighterSnapshot::default()
        });

        ai.begin_cassos_panic(252, &ctx, &tick);

        let panic = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("directed Cassos must stage Panic");
        assert_eq!(panic.center, Some(live));
        assert!(ai.base.directed_panic);
    }

    #[test]
    fn cassos_without_a_selected_target_uses_undirected_panic() {
        let mut ai = EnemyAi::new(117);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingPanic;
        ai.base.lasting_panic_runs = 11;
        ai.base.seek_position = Position {
            x: 2145.0,
            y: 1976.0,
            ..Position::default()
        };

        ai.begin_cassos_panic(0, &AiContext::default(), &AiPerTickData::stub());

        let panic = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("undirected Cassos must stage Panic");
        assert_eq!(panic.center, None);
        assert!(!ai.base.directed_panic);
        assert_eq!(panic.runs, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
        assert_eq!(
            ai.base.lasting_panic_runs, 11,
            "the engine drain owns repeated-panic upgrade semantics"
        );
    }

    #[test]
    #[should_panic(expected = "required entity view for handle 252 missing")]
    fn cassos_does_not_replace_a_missing_live_target_with_fighter_or_seek_position() {
        let mut ai = EnemyAi::new(117);
        ai.base.seek_position = Position {
            x: 2145.0,
            y: 1976.0,
            ..Position::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters.push(FighterSnapshot {
            handle: 252,
            position: Position {
                x: 900.0,
                y: 1800.0,
                ..Position::default()
            },
            ..FighterSnapshot::default()
        });

        ai.begin_cassos_panic(252, &AiContext::default(), &tick);
    }

    #[test]
    fn successful_look_for_help_continuation_draws_remark_and_logs_once() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(105);
        let mut global = AiGlobalState::default();

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            ai.resume_battle_look_for_help_after_alert_officer(
                &sim,
                &mut global,
                &AiContext::default(),
                &AiPerTickData::stub(),
            );
        });

        assert_eq!(draws, vec![crate::sim_rng::RngSite::BattlePanicRemark]);
        assert_eq!(ai.base.ai_log.len(), 1);
        let log = ai.base.ai_log.last().expect("LookForHelp decision log");
        assert_eq!(log.line_type, LogLineType::BattleDecision);
        assert_eq!(log.info, Decision::LookForHelp as u16);
    }

    #[test]
    fn reconsider_approach_uses_raw_truncated_map_distance() {
        let soldier = Position {
            x: 655.007_8,
            y: 1744.445,
            ..Position::default()
        };
        let target = Position {
            x: 585.0,
            y: 1726.0,
            ..Position::default()
        };

        assert_eq!(reconsider_approach_distance(soldier, target), 72.0);
        let dx = soldier.x - target.x;
        let dy = (soldier.y - target.y) * INVERSE_ASPECT_RATIO;
        assert!(
            (dx * dx + dy * dy).sqrt() > 75.0,
            "the general aspect-corrected distance would miss this swordfight boundary"
        );
    }

    #[test]
    fn battle_friend_visibility_is_evaluated_at_the_decision_call_site() {
        let mut target = pc_view();
        target.position.x = 100.0;
        target.detection_position_world.x = 100.0;
        let ctx = AiContext {
            self_view_radius: 500,
            self_is_active: true,
            ..AiContext::default()
        };

        assert!(battle_friend_detected_360(
            &ctx,
            1,
            2,
            target.detection_position_world,
            target.direction,
            &target,
        ));

        target.in_building = true;
        assert!(!battle_friend_detected_360(
            &ctx,
            1,
            2,
            target.detection_position_world,
            target.direction,
            &target,
        ));

        target.in_building = false;
        target.active = false;
        assert!(!battle_friend_detected_360(
            &ctx,
            1,
            2,
            target.detection_position_world,
            target.direction,
            &target,
        ));

        target.active = true;
        let inactive_owner = AiContext {
            self_is_active: false,
            ..ctx
        };
        assert!(!battle_friend_detected_360(
            &inactive_owner,
            1,
            2,
            target.detection_position_world,
            target.direction,
            &target,
        ));
    }

    #[test]
    fn battle_fighter_scan_preserves_interleaved_registry_order() {
        let fighter = |handle, is_pc, is_friendly, is_able_to_fight| FighterSnapshot {
            handle,
            is_pc,
            is_soldier: !is_pc,
            is_friendly,
            is_able_to_fight,
            ..FighterSnapshot::default()
        };
        let registry = vec![
            fighter(54, false, true, true),
            fighter(47, false, true, true),
            fighter(167, true, true, true),
            fighter(48, false, true, true),
            fighter(36, true, false, true),
            fighter(49, false, true, false),
        ];

        assert_eq!(
            battle_fighter_candidates(&registry, 54)
                .map(|candidate| candidate.handle)
                .collect::<Vec<_>>(),
            vec![47, 167, 48],
        );
    }

    #[test]
    fn out_of_view_alerting_soldier_does_not_suppress_officer_alert() {
        assert!(!has_nearby_alerting_soldier(
            65,
            &[65],
            [(64, Substate::SeekingRunningToOfficer)],
        ));
    }

    #[test]
    fn admitted_alerting_soldier_suppresses_duplicate_officer_alert() {
        assert!(has_nearby_alerting_soldier(
            65,
            &[65, 64],
            [(64, Substate::SeekingRunningToOfficer)],
        ));
        assert!(!has_nearby_alerting_soldier(
            65,
            &[65, 64],
            [(64, Substate::DefaultOnPost)],
        ));
    }

    #[test]
    fn rider_charge_approach_focuses_target_for_immediate_visibility_refresh() {
        let mut ai = EnemyAi::new(51);
        ai.base.primary_target = 76;

        let ctx = AiContext {
            self_is_rider: true,
            position: Position::default(),
            direction: 0,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![FighterSnapshot {
            handle: 76,
            position: Position {
                y: -200.0,
                ..Position::default()
            },
            raw_position: Position {
                y: -200.0,
                ..Position::default()
            },
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        }];

        assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRiderChargingApproaching
        );
        let focus = ai.base.outbox.actor.focus.or_else(|| {
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .and_then(|effects| effects.focus),
                    _ => None,
                })
        });
        assert_eq!(focus, Some(76));
    }

    #[test]
    fn immediate_rider_charge_replaces_target_focus_with_unfocus() {
        let mut ai = EnemyAi::new(51);
        ai.base.primary_target = 76;

        let ctx = AiContext {
            self_is_rider: true,
            position: Position::default(),
            direction: 0,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![FighterSnapshot {
            handle: 76,
            position: Position {
                y: -50.0,
                ..Position::default()
            },
            raw_position: Position {
                y: -50.0,
                ..Position::default()
            },
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        }];

        assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRiderChargingPassing
        );
        assert_eq!(ai.base.outbox.actor.focus, None);
        let unfocus = ai.base.outbox.actor.unfocus
            || ai
                .base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .is_some_and(|effects| effects.unfocus),
                    _ => false,
                });
        assert!(unfocus);
    }

    #[test]
    fn sleeping_enemy_visibility_is_evaluated_only_by_the_fallback() {
        let mut target = pc_view();
        target.position.x = 100.0;
        target.is_unconscious = true;
        let candidate = SleepingEnemyInfo {
            handle: 198,
            position: target.position,
            is_pc: true,
            is_robin: false,
            is_vip: false,
        };
        let ctx = AiContext {
            self_view_radius: 500,
            ..AiContext::default()
        };

        assert!(sleeping_enemy_detected_360(&ctx, &candidate, &target));
        target.in_building = true;
        assert!(!sleeping_enemy_detected_360(&ctx, &candidate, &target));
    }

    fn sleeping_target_case(
        first: Position,
        second: Position,
        expected: HumanHandle,
    ) -> (EnemyAi, AiContext) {
        let mut first_view = pc_view_at(first);
        first_view.elevation = 0.0;
        first_view.is_unconscious = true;
        let mut second_view = pc_view_at(second);
        second_view.elevation = 0.0;
        second_view.is_unconscious = true;
        let owner_position = Position {
            x: 1377.2015,
            y: 252.88869,
            sector: crate::position_interface::SectorHandle::new(14),
            ..Position::default()
        };
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(346, first_view);
        views.insert(345, second_view);
        views.insert(139, pc_view_at(owner_position));
        let ctx = AiContext {
            position: owner_position,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let targets = [
            SleepingEnemyInfo {
                handle: 346,
                position: first,
                is_pc: true,
                is_robin: false,
                is_vip: false,
            },
            SleepingEnemyInfo {
                handle: 345,
                position: second,
                is_pc: true,
                is_robin: false,
                is_vip: false,
            },
        ];
        let mut ai = EnemyAi::new(139);
        ai.approach_sleeping_enemies(
            &crate::sim_rng::test_context(),
            &targets,
            &ctx,
            &AiPerTickData::stub(),
        );

        assert_eq!(ai.base.primary_target, expected);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingApproachingSleepingEnemy
        );
        let order = ai
            .base
            .outbox
            .actor
            .orders
            .last()
            .expect("sleeping target selection must queue GoNear");
        let target = ctx.entity_view(expected).unwrap().position;
        assert_eq!((order.target_x, order.target_y), (target.x, target.y));
        (ai, ctx)
    }

    #[test]
    fn sleeping_enemy_selection_uses_isometric_get_position_distance() {
        let raw_nearer = Position {
            x: 1394.2125,
            y: 328.31696,
            sector: crate::position_interface::SectorHandle::new(14),
            ..Position::default()
        };
        let isometric_nearer = Position {
            x: 1417.7587,
            y: 185.4791,
            sector: crate::position_interface::SectorHandle::new(14),
            ..Position::default()
        };

        let owner = Position {
            x: 1377.2015,
            y: 252.88869,
            ..Position::default()
        };
        let raw_sq = |target: Position| {
            let dx = target.x - owner.x;
            let dy = target.y - owner.y;
            dx * dx + dy * dy
        };
        let isometric_sq = |target: Position| {
            let dx = target.x - owner.x;
            let dy = (target.y - owner.y) * INVERSE_ASPECT_RATIO;
            dx * dx + dy * dy
        };
        assert!(raw_sq(raw_nearer) < raw_sq(isometric_nearer));
        assert!(isometric_sq(isometric_nearer) < isometric_sq(raw_nearer));

        let _ = sleeping_target_case(raw_nearer, isometric_nearer, 345);
    }

    #[test]
    fn sleeping_enemy_selection_keeps_ordinary_nearest_target() {
        let nearest = Position {
            x: 1417.0,
            y: 250.0,
            sector: crate::position_interface::SectorHandle::new(14),
            ..Position::default()
        };
        let farther = Position {
            x: 1500.0,
            y: 400.0,
            sector: crate::position_interface::SectorHandle::new(14),
            ..Position::default()
        };

        let _ = sleeping_target_case(nearest, farther, 346);
    }

    #[test]
    fn reconsider_approach_resolves_position_after_synchronous_retarget() {
        let mut ai = EnemyAi::new(110);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.primary_target = 91;
        ai.sword_range = 50;

        let target_position = Position {
            x: 695.0,
            y: 2073.0,
            ..Position::default()
        };
        let mut target_view = pc_view();
        target_view.position = target_position;
        target_view.forecasted_destination =
            crate::ai::PreparedForecastDestination::fixed(target_position, 0);
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(91, target_view);
        let ctx = AiContext {
            position: Position {
                x: 698.0,
                y: 2119.0,
                ..Position::default()
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        add_owner_sword_range(&mut tick, 110, 50);
        tick.primary_target_snapshot_handle = 58;
        tick.primary_target_position = Some(Position {
            x: 712.0,
            y: 2053.0,
            ..Position::default()
        });

        ai.reconsider_enemy_approach(false, 0.0, &ctx, &tick, None);

        // begin_swordfight raises Engage before its SetState suspends the
        // actor-outbox prefix into the queued state-change owner work; the
        // engine reapplies that prefix when it drains the callback. Read the
        // request from either place.
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
        assert_eq!(engage, Some(EnterSwordfightRequest::Engage(91)));
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    }

    #[test]
    fn reconsider_approach_uses_selected_door_lift_after_synchronous_retarget() {
        let mut ai = EnemyAi::new(84);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingQuittingSwordfight;
        ai.base.primary_target = 173;
        ai.sword_range = 50;

        let committed_lift_position = Position {
            x: 500.0,
            y: 500.0,
            sector: crate::position_interface::SectorHandle::new(42),
            level: 0,
        };
        let mut replacement_view = pc_view();
        // A selected PassDoor makes AI Position(target) report the committed
        // endpoint while literal GetPosition remains at the interpolated body.
        // Poison the latter so this test cannot pass through raw geometry.
        replacement_view.position = committed_lift_position;
        replacement_view.detection_position = crate::coordinates::MapPoint::new(70.0, 80.0);
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(173, replacement_view);
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                level: 3,
                ..Position::default()
            },
            frame: 1058,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            fast_grid: std::sync::Arc::new(reconsider_approach_lift_grid()),
            ..AiContext::default()
        };
        assert_eq!(
            ctx.entity_view(173).unwrap().detection_position,
            crate::coordinates::MapPoint::new(70.0, 80.0)
        );
        assert_eq!(
            ctx.entity_view(173).unwrap().position,
            committed_lift_position
        );

        let mut tick = AiPerTickData::stub();
        add_owner_sword_range(&mut tick, 84, 50);
        tick.owner_live_position = Some(ctx.position);
        tick.primary_target_snapshot_handle = 47;
        tick.primary_target_position = Some(Position {
            x: 900.0,
            y: 900.0,
            sector: crate::position_interface::SectorHandle::new(5),
            level: 0,
        });

        ai.reconsider_enemy_approach(false, 0.0, &ctx, &tick, None);

        let expected_entry = Position {
            x: 410.0,
            y: 120.0,
            sector: crate::position_interface::SectorHandle::new(7),
            level: 3,
        };
        assert_eq!(ai.base.primary_target, 173);
        let focused = ai.base.outbox.actor.focus.or_else(|| {
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .and_then(|effects| effects.focus),
                    _ => None,
                })
        });
        assert_eq!(focused, Some(173));
        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToLadder);
        assert_eq!(ai.base.seek_position, expected_entry);
        assert_eq!(ai.base.outbox.actor.orders.len(), 1);
        let order = &ai.base.outbox.actor.orders[0];
        assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
        assert_eq!((order.target_x, order.target_y), (410.0, 120.0));
        assert_eq!(order.target_sector, expected_entry.sector);
        assert_eq!(order.target_layer, Some(3));
        assert_eq!(order.tolerance, 30.0);
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 1088);
    }

    #[test]
    fn reconsider_approach_does_not_reuse_old_lift_after_synchronous_retarget() {
        let mut ai = EnemyAi::new(84);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingQuittingSwordfight;
        ai.base.primary_target = 173;
        ai.sword_range = 50;

        let ordinary_replacement = Position {
            x: 700.0,
            y: 0.0,
            sector: crate::position_interface::SectorHandle::new(5),
            level: 0,
        };
        let mut replacement_view = pc_view();
        replacement_view.position = ordinary_replacement;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(173, replacement_view);
        let ctx = AiContext {
            position: Position::default(),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            fast_grid: std::sync::Arc::new(reconsider_approach_lift_grid()),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        // The serialized EnemyAi cache deliberately disagrees with the live
        // actor weapon. Original asks RHSword for the latter on every
        // ReconsiderEnemyApproach GoNear.
        add_owner_sword_range(&mut tick, 84, 65);
        tick.primary_target_snapshot_handle = 47;
        tick.primary_target_position = Some(Position {
            x: 500.0,
            y: 500.0,
            sector: crate::position_interface::SectorHandle::new(42),
            level: 0,
        });

        ai.reconsider_enemy_approach(false, 0.0, &ctx, &tick, None);

        assert_eq!(ai.base.primary_target, 173);
        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
        assert_ne!(ai.base.current_substate, Substate::AttackingRunningToLadder);
        assert_eq!(ai.base.seek_position, ordinary_replacement);
        let order = ai.base.outbox.actor.orders.first().or_else(|| {
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .and_then(|effects| effects.orders.first()),
                    crate::ai::AiOwnerWork::ActorEffects(effects) => effects.orders.first(),
                    _ => None,
                })
        });
        let order = order.expect("ordinary replacement must queue its running approach");
        assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
        assert_eq!((order.target_x, order.target_y), (700.0, 0.0));
        assert_eq!(order.target_sector, ordinary_replacement.sector);
        assert_eq!(order.tolerance, 65.0);
    }

    #[test]
    fn reconsider_approach_move_precedes_state_change_callback() {
        let mut ai = EnemyAi::new(180);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingTooProudToAttackApproach;
        ai.base.primary_target = 198;
        ai.base.think_recursion_depth = 1;
        ai.sword_range = 50;

        let target_position = Position {
            x: 1731.4956,
            y: 2379.8796,
            ..Position::default()
        };
        let ctx = AiContext {
            position: Position {
                x: 1773.7925,
                y: 2523.631,
                ..Position::default()
            },
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        add_owner_sword_range(&mut tick, 180, 50);
        tick.primary_target_snapshot_handle = 198;
        tick.primary_target_position = Some(target_position);

        ai.reconsider_enemy_approach(true, 0.0, &ctx, &tick, None);

        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
        let transition = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification)
                    if notification.incoming_substate == Substate::AttackingRunningToEnemy =>
                {
                    Some(notification)
                }
                _ => None,
            })
            .expect("running approach must queue its state-change callback");
        assert!(transition.actor_effects_before_callback.is_none());
        let work = &ai.base.outbox.reentrant.owner_work;
        let actor_effects_index = work
            .iter()
            .position(|work| matches!(work, crate::ai::AiOwnerWork::ActorEffects(_)))
            .expect("GoNear must be sealed as an actor-effects owner boundary");
        let resume_index = work
            .iter()
            .position(|work| {
                matches!(
                    work,
                    crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear { .. }
                )
            })
            .expect("failed-route continuation must remain queued");
        assert!(actor_effects_index < resume_index);
        let state_change_index = work
            .iter()
            .position(|work| matches!(work, crate::ai::AiOwnerWork::StateChange(_)))
            .expect("approach SetState remains queued after its route prefix");
        assert!(actor_effects_index < state_change_index);
        let crate::ai::AiOwnerWork::ActorEffects(prefix) = &work[actor_effects_index] else {
            unreachable!()
        };
        assert_eq!(prefix.orders.len(), 1);
        assert_eq!(
            prefix.orders[0].order_type,
            crate::order::OrderType::RunningUpright
        );
        assert_eq!(prefix.orders[0].tolerance, 50.0);
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(
            work[state_change_index + 1..resume_index]
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::ActorEffects(effects)
                        if effects.set_attentive_mode.map(|effect| effect.target) == Some(true)
                ))
        );
        assert!(
            ai.base
                .outbox
                .reentrant
                .reconsider_approach_completion_pending
        );
        assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear {
                    target: 198,
                    target_position: queued_target,
                } if *queued_target == target_position
            )
        }));
    }

    #[test]
    fn reconsider_approach_same_substate_seals_live_move_without_stealing_old_callback() {
        let mut ai = EnemyAi::new(180);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 198;
        ai.base.think_recursion_depth = 1;
        ai.sword_range = 50;
        ai.base
            .outbox
            .reentrant
            .owner_work
            .push(crate::ai::AiOwnerWork::StateChange(
                crate::ai::AiStateChangeNotification {
                    outgoing_state: AiState::Attacking,
                    outgoing_substate: Substate::AttackingTooProudToAttackApproach,
                    incoming_state: AiState::Attacking,
                    incoming_substate: Substate::AttackingRunningToEnemy,
                    source: crate::ai::AiStateChangeSource::from_optional_human(198),
                    actor_effects_before_callback: None,
                },
            ));

        let target_position = Position {
            x: 1731.4956,
            y: 2379.8796,
            ..Position::default()
        };
        let ctx = AiContext {
            position: Position {
                x: 1773.7925,
                y: 2523.631,
                ..Position::default()
            },
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        add_owner_sword_range(&mut tick, 180, 50);
        tick.primary_target_snapshot_handle = 198;
        tick.primary_target_position = Some(target_position);

        ai.reconsider_enemy_approach(true, 0.0, &ctx, &tick, None);

        let work = &ai.base.outbox.reentrant.owner_work;
        let crate::ai::AiOwnerWork::StateChange(old_notification) = &work[0] else {
            panic!("older matching callback must retain its owner slot")
        };
        assert!(old_notification.actor_effects_before_callback.is_none());
        assert_eq!(
            work.iter()
                .filter(|work| matches!(work, crate::ai::AiOwnerWork::StateChange(_)))
                .count(),
            1,
            "same-substate SetState must not manufacture a callback"
        );
        let actor_effects_index = work
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(index, work)| match work {
                crate::ai::AiOwnerWork::ActorEffects(effects)
                    if effects.orders.iter().any(|order| {
                        order.order_type == crate::order::OrderType::RunningUpright
                            && order.tolerance == 50.0
                    }) =>
                {
                    Some(index)
                }
                _ => None,
            })
            .expect("same-substate GoNear must become real actor owner work");
        let resume_index = work
            .iter()
            .position(|work| {
                matches!(
                    work,
                    crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear { .. }
                )
            })
            .expect("route completion must resume the source statement");
        assert!(actor_effects_index < resume_index);
        let attentive_effects_index = work
            .iter()
            .enumerate()
            .skip(actor_effects_index + 1)
            .find_map(|(index, work)| match work {
                crate::ai::AiOwnerWork::ActorEffects(effects)
                    if effects.set_attentive_mode.map(|effect| effect.target) == Some(true) =>
                {
                    Some(index)
                }
                _ => None,
            })
            .expect("same-substate SetState tail must remain after the GoNear boundary");
        assert!(attentive_effects_index < resume_index);
        let crate::ai::AiOwnerWork::ActorEffects(route_effects) = &work[actor_effects_index] else {
            unreachable!()
        };
        assert!(route_effects.set_attentive_mode.is_none());
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(
            ai.base
                .outbox
                .reentrant
                .reconsider_approach_completion_pending
        );
    }

    #[test]
    fn failed_reconsider_approach_resumes_with_avenger_roof_wait() {
        let mut ai = EnemyAi::new(205);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 298;
        ai.base.couldnt_reachpoint = true;
        let target_position = Position {
            x: 264.0,
            y: 1358.0,
            ..Position::default()
        };
        let wait_position = Position {
            x: 250.0,
            y: 1200.0,
            sector: crate::position_interface::SectorHandle::new(64),
            level: 1,
        };

        ai.resume_reconsider_enemy_approach_after_go_near(
            target_position,
            Some(wait_position),
            &AiContext::default(),
        );

        assert!(!ai.base.couldnt_reachpoint);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunToAvengerOnRoof
        );
        assert_eq!(ai.base.seek_position, target_position);
        assert_eq!(ai.base.outbox.actor.orders.len(), 1);
        let order = &ai.base.outbox.actor.orders[0];
        assert_eq!(order.target_x, wait_position.x);
        assert_eq!(order.target_y, wait_position.y);
        assert_eq!(order.target_sector, wait_position.sector);
        assert_eq!(order.target_layer, Some(wait_position.level));
        assert_eq!(order.tolerance, 50.0);
        assert!(
            !order.defer_instruction,
            "an ordinary synchronous route failure instructs its roof fallback this frame"
        );
    }

    #[test]
    fn close_avenger_roof_wait_position_completes_without_an_order() {
        let mut ai = EnemyAi::new(205);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 298;
        ai.base.couldnt_reachpoint = true;
        ai.base.think_recursion_depth = 1;
        ai.base
            .outbox
            .reentrant
            .reconsider_approach_replaced_path_waiter = true;
        let target_position = Position {
            x: 264.0,
            y: 1358.0,
            ..Position::default()
        };
        let wait_position = Position {
            x: 250.0,
            y: 1200.0,
            sector: crate::position_interface::SectorHandle::new(64),
            level: 1,
        };
        let ctx = AiContext {
            position: wait_position,
            self_layer: wait_position.level,
            ..AiContext::default()
        };

        ai.resume_reconsider_enemy_approach_after_go_near(
            target_position,
            Some(wait_position),
            &ctx,
        );

        assert!(!ai.base.couldnt_reachpoint);
        assert!(ai.base.already_on_point);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunToAvengerOnRoof
        );
        assert_eq!(ai.base.seek_position, target_position);
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(
            !ai.base
                .outbox
                .reentrant
                .reconsider_approach_replaced_path_waiter
        );
    }

    #[test]
    fn failed_reconsider_approach_replacing_path_waiter_halts_roof_after_launch() {
        let mut ai = EnemyAi::new(205);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 298;
        ai.base.couldnt_reachpoint = true;
        ai.base
            .outbox
            .reentrant
            .reconsider_approach_replaced_path_waiter = true;

        ai.resume_reconsider_enemy_approach_after_go_near(
            Position {
                x: 264.0,
                y: 1358.0,
                ..Position::default()
            },
            Some(Position {
                x: 250.0,
                y: 1200.0,
                sector: crate::position_interface::SectorHandle::new(64),
                level: 1,
            }),
            &AiContext::default(),
        );

        assert!(ai.base.outbox.actor.orders[0].halt_after_launch_for_path_waiter);
        assert!(!ai.base.outbox.actor.orders[0].defer_instruction);
        assert!(
            !ai.base
                .outbox
                .reentrant
                .reconsider_approach_replaced_path_waiter,
            "path-waiter provenance is one-shot"
        );
    }

    #[test]
    fn reconsider_approach_already_near_engages_before_approach_state_change() {
        let mut ai = EnemyAi::new(180);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.primary_target = 198;
        ai.base.think_recursion_depth = 1;
        ai.sword_range = 150;
        ai.sword_is_charge_weapon = true;

        let target_position = Position {
            x: 100.0,
            ..Position::default()
        };
        let ctx = AiContext::default();
        let mut tick = AiPerTickData::stub();
        add_owner_sword_range(&mut tick, 180, 150);
        tick.primary_target_snapshot_handle = 198;
        tick.primary_target_position = Some(target_position);
        // Preserve the charge branch past Original's intentionally broad
        // "walking circus pyramid" command comparison.
        tick.primary_target_animation = Some(crate::order::OrderType::WalkingCarryingOnShoulders);

        ai.reconsider_enemy_approach(false, 0.0, &ctx, &tick, None);

        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert!(!ai.base.already_on_point);
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(
            !ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::StateChange(notification)
                        if notification.incoming_substate
                            == Substate::AttackingChargingEnemy
                )),
            "the already-near branch must engage before SetState(ChargingEnemy)"
        );
    }

    #[test]
    fn observe_threshold_keeps_fractional_courage_bonus() {
        // One visible enemy and courage 45 yields 3.025 in Original. Three
        // nearer friends are therefore insufficient; four are sufficient.
        assert!(!enough_nearer_friends_to_observe(3, 1, 45));
        assert!(enough_nearer_friends_to_observe(4, 1, 45));
    }

    #[test]
    fn elevated_owner_distance_does_not_count_two_door_friends_as_nearer() {
        // linux2/Profile_002/Savegame_001/replay-001, immediately before
        // frame 2147. Soldier 137 and PC 252 are separated vertically, so
        // Original's literal 3D SquareDistance is much smaller than the
        // distance obtained after projecting both actors to map space.
        let owner_world = crate::coordinates::WorldPoint3D::new(1720.6782, 2002.2788, 17.413866);
        let target_world = crate::coordinates::WorldPoint3D::new(1741.3412, 2000.2783, 37.16403);
        let owner_target_sq = battle_owner_target_square_distance(owner_world, target_world);
        assert_eq!(owner_target_sq, 829);

        let target = Position {
            x: 1741.3412,
            y: 1963.1143,
            ..Position::default()
        };
        // Soldiers 104 and 142 are both passing door 100 directly. AI
        // Position() commits both to point_in=(1712, 1994), whose raw map
        // distance is outside the correct 829 threshold but inside the old
        // projected-map threshold (~1865).
        let door_point_in = Position {
            x: 1712.0,
            y: 1994.0,
            ..Position::default()
        };
        assert!(!battle_friend_is_nearer(
            door_point_in,
            target,
            owner_target_sq
        ));

        let ordinary_nearer_friend = Position {
            x: 1722.0557,
            y: 1983.415,
            ..Position::default()
        };
        let mut nearer_friends = 1_u16; // Soldier 139 is already swordfighting.
        for friend in [ordinary_nearer_friend, door_point_in, door_point_in] {
            if battle_friend_is_nearer(friend, target, owner_target_sq) {
                nearer_friends += 1;
            }
        }
        assert_eq!(nearer_friends, 2);

        let decision = if enough_nearer_friends_to_observe(nearer_friends, 1, 40) {
            Decision::Observe
        } else {
            Decision::Fight
        };
        assert_eq!(decision, Decision::Fight);
    }

    #[test]
    fn reserve_cutoff_uses_literal_3d_square_distance() {
        // linux3/Profile_003/Savegame_010/replay-003, frame 19649.
        // Projecting the actors to map space puts Soldier 144 just outside
        // the 150-unit reserve radius, while Original's literal 3D
        // GetPosition() distance puts it inside and falls through to Observe.
        let owner_world = crate::coordinates::WorldPoint3D::new(669.78064, 1511.1458, 35.611286);
        let target_world = crate::coordinates::WorldPoint3D::new(745.8495, 1441.5433, 50.001003);
        let literal_3d = battle_owner_target_square_distance(owner_world, target_world);
        assert!(literal_3d < combat::MIN_SQUARE_RESERVE_DISTANCE as u32);

        let dx = 745.8495_f32 - 669.78064_f32;
        let dy = (1391.5424_f32 - 1475.5344_f32) * INVERSE_ASPECT_RATIO;
        let projected_map = (dx * dx + dy * dy) as u32;
        assert!(projected_map > combat::MIN_SQUARE_RESERVE_DISTANCE as u32);
    }

    #[test]
    fn observe_move_precedes_state_change_callback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontimeRunning;
        ai.list_them = vec![198];
        ai.base.stop_all();

        let target_position = Position {
            x: 500.0,
            ..Position::default()
        };
        let target_view = pc_view_at(target_position);
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(198, target_view);
        views.insert(91, pc_view_at(Position::default()));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        // Deliberately leave nearby_fighters empty: the persistent Them-list
        // target can lie just outside that 500-unit decision snapshot.
        let tick = AiPerTickData::stub();

        assert!(!ai.execute_battle_decision(
            &sim,
            Decision::Observe,
            Substate::AttackingReactiontimeRunning,
            0,
            &mut std::collections::BTreeMap::from([(198, 0)]),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        ));

        let [
            crate::ai::AiOwnerWork::ActorEffects(route),
            crate::ai::AiOwnerWork::ResumeBattleObserveAfterGoNear {
                target,
                target_position: queued_target_position,
            },
        ] = ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!(
                "routed Observe must settle movement before resuming its source-ordered tail: {:?}",
                ai.base.outbox.reentrant.owner_work
            );
        };
        assert!(route.halt);
        assert_eq!(route.orders.len(), 1);
        assert_eq!(
            route.orders[0].order_type,
            crate::order::OrderType::WalkingUpright
        );
        assert_eq!(*target, 198);
        assert_eq!(*queued_target_position, target_position);
        assert!(ai.base.outbox.reentrant.battle_observe_completion_pending);
        assert!(ai.base.outbox.actor.orders.is_empty());
        // `battle_observe_route_settles_before_source_ordered_tail` exercises
        // the engine drain that consumes this continuation and performs the
        // following SetState callback at Original's synchronous boundary.
    }

    #[test]
    fn failed_fight_approach_resumes_inline_observe_decision() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = 198;
        ai.base.couldnt_reachpoint = true;
        ai.list_them = vec![198];

        let target_position = Position {
            x: 500.0,
            ..Position::default()
        };
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(198, pc_view_at(target_position));
        views.insert(91, pc_view_at(Position::default()));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.resume_battle_fight_after_reconsider(
            &sim,
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
        );

        assert!(!ai.base.couldnt_reachpoint);
        assert_eq!(ai.base.primary_target, 198);
        let [
            crate::ai::AiOwnerWork::ActorEffects(route),
            crate::ai::AiOwnerWork::ResumeBattleObserveAfterGoNear {
                target,
                target_position: queued_target_position,
            },
        ] = ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!(
                "failed Fight must continue through Observe on the same owner boundary: {:?}",
                ai.base.outbox.reentrant.owner_work
            );
        };
        assert_eq!(*target, 198);
        assert_eq!(*queued_target_position, target_position);
        assert_eq!(route.orders.len(), 1);
        assert_eq!(
            route.orders[0].order_type,
            crate::order::OrderType::WalkingUpright
        );
        assert!(ai.base.outbox.reentrant.battle_observe_completion_pending);
    }

    fn proud_decision_speech(
        entry_substate: Substate,
        serialized_previous_substate: Substate,
    ) -> Vec<Remark> {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = entry_substate;
        ai.previous_substate = serialized_previous_substate as i32;
        ai.forced_next_battle_decision = Decision::TooProudToAttack;
        ai.list_them = vec![198];

        let target_position = Position {
            x: 150.0,
            ..Position::default()
        };
        let mut target_view = pc_view();
        target_view.position = target_position;
        target_view.forecasted_destination =
            crate::ai::PreparedForecastDestination::fixed(target_position, 0);
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        let mut owner_view = pc_view();
        owner_view.is_pc = false;
        owner_view.kind = crate::ai_entity_view::EntityKind::Soldier;
        owner_view.camp = crate::element::Camp::Lacklandists;
        views.insert(91, owner_view);
        views.insert(198, target_view);
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.enemy_sq_distances = vec![(198, 150 * 150)];
        tick.nearby_fighters = vec![FighterSnapshot {
            handle: 198,
            position: target_position,
            is_able_to_fight: true,
            is_pc: true,
            ..Default::default()
        }];

        ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .filter_map(|work| match work {
                crate::ai::AiOwnerWork::Speech(attempt) => Some(attempt.remark),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn proud_first_decision_uses_entry_substate_not_serialized_previous_substate() {
        assert_eq!(
            proud_decision_speech(Substate::AttackingReactiontime, Substate::DefaultOnPost,),
            vec![Remark::ProudDontFight]
        );
    }

    #[test]
    fn proud_later_decision_ignores_stale_reactiontime_previous_substate() {
        assert!(
            proud_decision_speech(
                Substate::AttackingTooProudToAttack,
                Substate::AttackingReactiontime,
            )
            .is_empty()
        );
    }

    #[test]
    fn alert_soldiers_without_a_live_target_falls_back_to_reserve() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;

        assert!(ai.execute_battle_decision(
            &sim,
            Decision::AlertSoldiers,
            Substate::AttackingReactiontime,
            0,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        ));

        assert_eq!(ai.base.primary_target, 0);
        assert!(!ai.base.friends_are_alerted);
        assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
        assert!(ai.base.timer_is_running);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    }

    #[test]
    fn tower_guard_decisions_without_a_live_target_fall_back_to_reserve() {
        let sim = crate::sim_rng::test_context();

        for decision in [Decision::TowerGuardAlert, Decision::TowerGuardObserve] {
            let mut ai = EnemyAi::new(91);
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingReactiontime;

            assert!(ai.execute_battle_decision(
                &sim,
                decision,
                Substate::AttackingReactiontime,
                0,
                &mut std::collections::BTreeMap::new(),
                &mut AiGlobalState::default(),
                &AiContext::default(),
                &AiPerTickData::stub(),
                None,
            ));

            assert_eq!(ai.base.primary_target, 0, "{decision:?}");
            assert!(!ai.base.friends_are_alerted, "{decision:?}");
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingReserve,
                "{decision:?}"
            );
            assert!(ai.base.timer_is_running, "{decision:?}");
            assert!(ai.base.outbox.actor.orders.is_empty(), "{decision:?}");
        }
    }

    #[test]
    fn archer_step_back_without_a_live_target_falls_back_through_shoot() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        let ctx = AiContext {
            remaining_arrows: 1,
            ..AiContext::default()
        };

        assert!(ai.execute_battle_decision(
            &sim,
            Decision::ArcherStepBack,
            Substate::AttackingReactiontime,
            0,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ));

        assert_eq!(ai.base.primary_target, 0);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingBowObservingLoading
        );
    }

    #[test]
    fn tower_guard_uses_live_ai_position_instead_of_stale_nearby_snapshot() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.owner_entity_id = Some(crate::element::EntityId::Soldier(
            crate::element::SoldierId(91),
        ));
        ai.tower_guard = true;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.list_them = vec![198];
        ai.forced_next_battle_decision = Decision::TowerGuardAlert;

        let live_position = Position {
            x: 1386.0,
            y: 1356.0,
            sector: crate::position_interface::SectorHandle::new(19),
            level: 1,
        };
        let stale_position = Position {
            x: 1389.106,
            y: 1361.8235,
            ..live_position
        };
        let mut owner_view = pc_view();
        owner_view.is_pc = false;
        owner_view.kind = crate::ai_entity_view::EntityKind::Soldier;
        owner_view.camp = crate::element::Camp::Lacklandists;
        let mut target_view = pc_view();
        target_view.position = live_position;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(91, owner_view);
        views.insert(198, target_view);
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.enemy_sq_distances = vec![(198, 100)];
        tick.nearby_fighters = vec![FighterSnapshot {
            handle: 198,
            position: stale_position,
            is_able_to_fight: true,
            is_pc: true,
            ..Default::default()
        }];

        ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

        assert_eq!(ai.base.primary_target, 198);
        assert_eq!(ai.base.seek_position, live_position);
        assert_eq!(ai.base.current_substate, Substate::AttackingTowerGuardAlert);
    }

    fn battle_cleanup_context(
        target: crate::ai_entity_view::AiEntityView,
    ) -> (AiContext, AiPerTickData) {
        let mut owner = pc_view();
        owner.is_pc = false;
        owner.kind = crate::ai_entity_view::EntityKind::Soldier;
        owner.camp = crate::element::Camp::Lacklandists;
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(91, owner);
        views.insert(198, target);
        (
            AiContext {
                camp: crate::element::Camp::Lacklandists,
                frame: 700,
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            },
            AiPerTickData::stub(),
        )
    }

    #[test]
    fn stale_same_camp_them_entry_preserves_visible_count_for_reserve() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOverviewLookRight;
        ai.list_them = vec![198];
        ai.forced_next_battle_decision = Decision::Reserve;

        let mut stale_friend = pc_view();
        stale_friend.camp = crate::element::Camp::Lacklandists;
        let (ctx, tick) = battle_cleanup_context(stale_friend);

        ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

        assert!(ai.list_them.is_empty(), "the stale friend must be removed");
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 750);
    }

    #[test]
    fn unable_nonfriend_them_entry_consumes_visible_count_and_returns_to_duty() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOverviewLookRight;
        ai.list_them = vec![198];
        ai.forced_next_battle_decision = Decision::Reserve;

        let mut unable_enemy = pc_view();
        unable_enemy.camp = crate::element::Camp::Royalists;
        unable_enemy.is_able_to_fight = false;
        let (ctx, tick) = battle_cleanup_context(unable_enemy);

        ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

        assert!(ai.list_them.is_empty());
        assert_ne!(ai.base.current_substate, Substate::AttackingReserve);
        assert!(!ai.base.timer_is_running);
        assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }
            )
        }));
    }

    #[test]
    fn battle_decisions_preserves_enemy_list_from_last_explicit_rebuild() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.primary_target = 198;
        ai.list_them = vec![198, 199];

        // The predecision pass walks the persistent us-list through the
        // shared entity-view table, and that list always includes the
        // evaluating soldier itself.
        let me_entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                active: true,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: crate::element::NpcData {
                life_points: 50,
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                    ..Default::default()
                },
            },
            soldier: Default::default(),
        });
        let me_view = crate::ai_entity_view::entity_view_from_entity(
            &me_entity,
            40,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        );
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(91, me_view);
        views.insert(198, pc_view());
        views.insert(199, pc_view());
        assert!(views[&198].is_able_to_fight);
        assert!(views[&199].is_able_to_fight);
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.enemy_sq_distances = vec![(198, 100)];
        tick.nearby_fighters = vec![
            FighterSnapshot {
                handle: 198,
                position: Position::default(),
                is_able_to_fight: true,
                is_pc: true,
                ..Default::default()
            },
            FighterSnapshot {
                handle: 199,
                position: Position {
                    x: 20.0,
                    ..Position::default()
                },
                is_able_to_fight: true,
                is_pc: true,
                ..Default::default()
            },
        ];

        ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

        assert!(
            ai.list_them.contains(&199),
            "BattleDecisions must not replace the persistent list with its tick snapshot"
        );
    }

    #[test]
    fn cover_behind_untargeted_shield_bearer_falls_back_to_archer_observe() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(69);
        ai.is_archer_unit = true;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontimeRunning;
        ai.base.primary_target = 126;

        let ctx = AiContext {
            remaining_arrows: 10,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![FighterSnapshot {
            handle: 73,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            primary_target: 0,
            ..FighterSnapshot::default()
        }];

        assert!(ai.execute_battle_decision(
            &sim,
            Decision::CoverBehindShieldBearer,
            Substate::AttackingReactiontimeRunning,
            73,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        ));

        assert_eq!(ai.shield_bearer_before_me, 0);
        assert_eq!(ai.base.primary_target, 0);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingBowObservingLoading
        );
        let mut launch_commands = Vec::new();
        for work in &ai.base.outbox.reentrant.owner_work {
            if let crate::ai::AiOwnerWork::StateChange(notification) = work
                && let Some(effects) = &notification.actor_effects_before_callback
            {
                launch_commands.extend(effects.launch_commands.iter().copied());
            }
        }
        launch_commands.extend(ai.base.outbox.actor.launch_commands.iter().copied());
        assert_eq!(launch_commands, vec![crate::element::Command::EquipBow]);
    }

    #[test]
    fn rejected_shield_cover_keeps_computed_seek_position_before_shoot_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(69);
        ai.is_archer_unit = true;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.primary_target = 126;
        ai.base.seek_position = Position {
            x: 10.0,
            y: 20.0,
            ..Position::default()
        };

        let bearer_position = Position {
            x: 200.0,
            y: 300.0,
            ..Position::default()
        };
        let target_position = Position {
            x: 1000.0,
            y: 1000.0,
            ..Position::default()
        };
        let ctx = AiContext {
            remaining_arrows: 10,
            sq_standard_view_radius: 100.0,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.nearby_fighters = vec![
            FighterSnapshot {
                handle: 73,
                position: bearer_position,
                direction: 0,
                is_friendly: true,
                is_soldier: true,
                is_shield_bearer: true,
                primary_target: 126,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 126,
                position: target_position,
                is_able_to_fight: true,
                is_pc: true,
                ..FighterSnapshot::default()
            },
        ];
        let expected_cover = ai
            .shield_bearer_cover_position(73, &tick)
            .expect("fixture shield bearer must produce a cover position");
        assert!(
            square_norm(pos_diff(&target_position, &expected_cover)) >= ctx.sq_standard_view_radius,
            "fixture must reject the computed cover point at the subsequent view-radius gate"
        );

        assert!(ai.execute_battle_decision(
            &sim,
            Decision::CoverBehindShieldBearer,
            Substate::AttackingReactiontime,
            73,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        ));

        assert_eq!(ai.base.seek_position, expected_cover);
        assert_eq!(ai.shield_bearer_before_me, 0);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingBowObservingLoading
        );
    }
}
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
    // BattleDecisions resets only the target already in its personal Them
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
    // Task #146: S131 resets PC101's shared UWORD before S178 appends that
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
    // BattleDecisions, but the archer's ProposeShotTarget reset the shared
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
    // PC173. An earlier same-frame AttackEnemy(174) observation must not
    // overwrite the later GetPrimaryTarget() value used by BattleDecisions.
    let swordfight_opponent = 174;
    let stale_attack_enemy_claim = swordfight_opponent;
    let live_primary_target = 173;
    let mut multiplicity =
        std::collections::BTreeMap::from([(live_primary_target, 0), (swordfight_opponent, 0)]);

    for _ in 0..2 {
        let target = battle_friend_primary_target(AiState::Attacking, live_primary_target)
            .expect("attacking friend has a live primary target");
        assert_ne!(target, stale_attack_enemy_claim);
        increment_battle_target_multiplicity(&mut multiplicity, target);
    }

    assert_eq!(multiplicity[&live_primary_target], 2);
    assert_eq!(multiplicity[&swordfight_opponent], 0);
}
