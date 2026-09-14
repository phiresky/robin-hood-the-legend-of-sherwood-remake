//! Combat positions, phalanx/shield-bearer formation, archery
//! shooting-point selection, and the swordfight repositioning loop.
//!
//! Owns the helpers used by `propose_good_combat_position`,
//! `reconsider_swordfight` and `reconsider_swordfight_observation`.
//! Also exposes `find_fighter`,
//! `is_allowed_to_attack`, and the neighbour predicates.

use crate::ai::*;
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::sim_rng::SimulationContext;

use super::map_vec_ext::AiMapVec;
use super::util::{
    ai_max_norm_distance, ai_square_distance, ai_square_distance_world, check_straight_movement,
    vec_to_sector,
};
use super::{
    CombatPosition, EnemyAi, FighterSnapshot, PrimaryTargetFlags, ProfileRank, Question, SeekFlags,
    ThinkEnv, UNDEFINED_DIRECTION, archer, combat, propose_good_step_back_goal,
};
use crate::coordinates::MapVec;

/// Us / them aggregates built by `reconsider_swordfight`.
#[derive(Clone, Copy)]
pub(crate) struct SwordfightLists {
    pub(crate) nearest_friend_solo: Option<AiEntityHandle>,
    pub(crate) number_of_swordfighting_enemies: u16,
    pub(crate) number_of_friends: u16,
}

/// Opt-in trace for the event-driven swordfight reposition decision. Keep the
/// gate process-local and evaluate it before touching any proposal data so the
/// disabled path cannot add lookups, RNG draws, or simulation state.
pub(super) fn reconsider_position_debug_matches(
    frame: impl FnOnce() -> u32,
    creation_order: impl FnOnce() -> Option<u32>,
    owner: impl FnOnce() -> u32,
) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_RECONSIDER_POSITION",
            [
                "PARITY_DEBUG_RECONSIDER_POSITION_FRAME",
                "PARITY_DEBUG_RECONSIDER_POSITION_CREATION_ORDER",
                "PARITY_DEBUG_RECONSIDER_POSITION_OWNER_HANDLE",
            ],
        )
    });
    gate.enabled() && gate.matches_required([Some(frame()), creation_order(), Some(owner())])
}

fn original_uword_norm(delta: MapVec) -> u16 {
    (delta.x * delta.x + delta.y * delta.y).sqrt() as u16
}

pub(crate) fn is_facing_swordfight_target(
    me_position: &Position,
    me_elevation: f32,
    me_direction: u16,
    target_position: &Position,
    target_elevation: f32,
) -> bool {
    // The original game compares ground-position values here. The position
    // stores projected map Y, so reconstruct ground/world Y by adding
    // elevation before selecting the aspect-corrected direction sector.
    let to_target = (
        target_position.x - me_position.x,
        (target_position.y + target_elevation) - (me_position.y + me_elevation),
    );
    let target_sector = vec_to_sector(to_target.0, to_target.1);
    let facing_delta = (me_direction as i32 + 16 - target_sector as i32).rem_euclid(16);
    matches!(facing_delta, 15 | 0 | 1)
}

#[cfg(test)]
fn swordfight_facing_target_position(
    primary: &FighterSnapshot,
    tick: &AiPerTickData,
    refreshed_live_position: impl FnOnce(HumanHandle) -> Position,
) -> Position {
    if tick.primary_target_snapshot_handle == Some(AiEntityHandle::new(primary.handle)) {
        tick.primary_target_live_position
            .unwrap_or(primary.position)
    } else {
        refreshed_live_position(primary.handle)
    }
}

/// The original game narrows combat-neighbour squared distance to an unsigned 32-bit value before
/// ranking. Reject corrupt/out-of-domain geometry explicitly instead of using
/// Rust's saturating float-to-integer cast, which could turn NaN into a
/// nearest-candidate distance of zero.
pub(crate) fn combat_neighbour_distance_ulong(distance: f32) -> u32 {
    assert!(
        distance.is_finite() && (0.0..4_294_967_296.0_f32).contains(&distance),
        "combat-neighbour squared distance {distance:?} is outside the original-game 32-bit unsigned domain"
    );
    distance as u32
}

impl EnemyAi {
    // -----------------------------------------------------------------------
    // Combat-position selection helpers
    // -----------------------------------------------------------------------

    /// Look up a fighter snapshot by handle in the engine-provided cache.
    pub(super) fn find_fighter<'a>(
        &self,
        handle: impl IntoOptionalAiHandle,
        tick: &'a AiPerTickData,
    ) -> Option<&'a FighterSnapshot> {
        let handle = handle.into_optional_ai_handle()?.get();
        tick.nearby_fighters
            .iter()
            .find(|f| f.handle == handle)
            .or_else(|| tick.fighter_registry.iter().find(|f| f.handle == handle))
    }

    /// [`Self::find_fighter`] for callers that tolerate a missing snapshot by
    /// taking a fallback branch: a null handle stays silent, but a non-null
    /// handle absent from both fighter lists is logged (with the calling
    /// site) so the unchanged fallback is visible.
    #[track_caller]
    pub(super) fn find_fighter_logged<'a>(
        &self,
        handle: impl IntoOptionalAiHandle,
        tick: &'a AiPerTickData,
        what: &'static str,
    ) -> Option<&'a FighterSnapshot> {
        let raw = handle.into_optional_ai_handle()?;
        let found = self.find_fighter(raw, tick);
        if found.is_none() {
            tracing::warn!(
                me = self.base.me,
                handle = raw.get(),
                what,
                caller = %std::panic::Location::caller(),
                "fighter snapshot unavailable; taking the caller's absent-fighter fallback"
            );
        }
        found
    }

    /// [`Self::find_fighter`] for a fighter the caller's precondition
    /// guarantees is present; panics with `context` (at the caller's
    /// location) when it is absent.
    #[track_caller]
    pub(super) fn required_fighter<'a>(
        &self,
        handle: impl IntoOptionalAiHandle,
        tick: &'a AiPerTickData,
        context: std::fmt::Arguments<'_>,
    ) -> &'a FighterSnapshot {
        self.find_fighter(handle, tick)
            .unwrap_or_else(|| panic!("{context}"))
    }

    /// Attack permission — VIP / mission rules.
    ///
    /// Pure VIP/Robin gate. Does NOT filter on friendliness or
    /// `is_able_to_fight` — those are caller responsibilities (the
    /// reference dereferences the caller-supplied pointer without those
    /// guards). Resolves the target via the broader `entity_view` map
    /// first so callers passing a handle outside the 500px
    /// `nearby_fighters` snapshot still get a meaningful answer.
    pub(super) fn is_allowed_to_attack(
        &self,
        target: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Prefer entity_view (broader population) for the VIP/Robin/PC
        // properties; fall back to the fighter snapshot if absent.
        let (target_is_pc, target_is_robin, target_is_vip) =
            if let Some(view) = ctx.entity_view(target) {
                (view.is_pc, view.is_robin, view.is_vip)
            } else if let Some(adversary) = self.find_fighter(target, tick) {
                (adversary.is_pc, adversary.is_robin, adversary.is_vip)
            } else {
                // No info available — the reference would dereference
                // the pointer (no guard) and assume the target is valid;
                // match that.
                tracing::warn!(
                    me = self.base.me,
                    target,
                    "is_allowed_to_attack: target not in entity_view or fighter snapshot"
                );
                return true;
            };

        // Rule 1: VIPs can only begin combat with Robin.
        if self.is_vip && (!target_is_pc || !target_is_robin) {
            return false;
        }

        // Rule 2: Soldiers cannot begin combat with VIP NPCs.
        if !target_is_pc && target_is_vip {
            return false;
        }

        true
    }

    // -----------------------------------------------------------------------
    // Phalanx / shield-bearer formation helpers
    // -----------------------------------------------------------------------

    /// Find the nearest free shield bearer. Scans the complete friendly-soldier
    /// registry for the nearest shield bearer already in (or heading into)
    /// a shield-bearer substate. Original does not check combat readiness here:
    /// an inactive or script-locked bearer remains a valid formation anchor.
    /// If the caller is a shield bearer any protecting shield bearer will do;
    /// if the caller is an archer we only accept shield bearers that don't yet
    /// have an archer behind them.
    pub(super) fn get_nearest_free_shield_bearer(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> Option<HumanHandle> {
        let me_snap = self.find_fighter(self.base.me, tick)?;
        let i_am_shield_bearer = me_snap.is_shield_bearer;

        let shield_running = crate::ai::Substate::AttackingRunningToPhalanx;
        let shield_phalanx = crate::ai::Substate::AttackingPhalanx;
        let shield_protecting = crate::ai::Substate::AttackingProtectingWithShield;

        let min_distance = archer::SHIELD_BEARER_MIN_DISTANCE as f32;
        let mut best = None;
        let mut best_distance = min_distance;

        for f in &tick.fighter_registry {
            if f.handle == self.base.me || !f.is_friendly || !f.is_shield_bearer {
                continue;
            }
            // If we're an archer, the shield bearer must not already
            // have someone hiding behind them.
            if !i_am_shield_bearer && f.archer_behind_me.is_some() {
                // This shield bearer already has an archer — skip.
                continue;
            }

            if f.current_substate != shield_running
                && f.current_substate != shield_phalanx
                && f.current_substate != shield_protecting
            {
                continue;
            }
            // The original game's maximum-norm distance subtracts the two raw
            // raw element positions. It does not call AI
            // `Position()`, which may snap a door-passing bearer to the
            // committed gate endpoint. Keep that accessor distinction here;
            // slot construction below still intentionally uses the bearer's
            // AI-facing position/seek position.
            let dist = ai_max_norm_distance(
                &f.raw_position,
                f.elevation,
                &me_snap.raw_position,
                me_snap.elevation,
            ) as u16;
            if crate::ai_enemy::battle_decision_debug_enabled() {
                crate::ai_enemy::parity_trace::ShieldBearerCandidate {
                    frame: &(ctx.frame),
                    me: &(self.base.me),
                    candidate: &(f.handle),
                    substate: &(f.current_substate as u32),
                    archer_behind: &(f.archer_behind_me),
                    dist: &(dist),
                    min_distance: &(min_distance),
                }
                .emit();
            }
            if f32::from(dist) < best_distance {
                best_distance = f32::from(dist);
                best = Some(f.handle);
            }
        }

        if crate::ai_enemy::battle_decision_debug_enabled() {
            let shield_bearers = tick
                .fighter_registry
                .iter()
                .filter(|f| f.is_shield_bearer)
                .map(|f| {
                    (
                        f.handle,
                        f.is_friendly,
                        f.current_substate,
                        f.archer_behind_me,
                    )
                })
                .collect::<Vec<_>>();
            crate::ai_enemy::parity_trace::ShieldBearerResult {
                frame: &(ctx.frame),
                me: &(self.base.me),
                registry: &(tick.fighter_registry.len()),
                best: &(best),
                shield_bearers: &(shield_bearers),
            }
            .emit();
        }
        best
    }

    /// Searches archery sectors for a shooting point that
    /// contains the primary target
    /// and isn't full, then finds the nearest free shooting point and
    /// nearest entry point. Sets up `my_archery_*` fields for the path.
    /// Returns `true` if a good shooting point was found.
    pub(super) fn choose_good_shooting_point(
        &mut self,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // (0) Clear the current shooting point. This also
        // releases the prior point's owner.
        self.set_my_shooting_point(global, None);

        // The reference implicitly requires a non-null primary target —
        // it would crash otherwise. Rather than falling back to
        // ctx.position (which meaninglessly tests "a point inside my own
        // sector"), bail out cleanly so the caller treats this as "no
        // good shooting point".
        let Some(primary) = self.find_fighter(self.base.primary_target, tick) else {
            tracing::trace!(
                me = self.base.me,
                primary_target = ?self.base.primary_target,
                "choose_good_shooting_point: primary target not visible; bailing"
            );
            return false;
        };
        let primary_pos = primary.position;

        // (1) Search for an archery sector containing the enemy.
        // The archery-sector containment test
        // rejects the sector when its own layer differs from
        // the enemy position's level—the layer travels with the enemy position the
        // caller passed in, not with the archer.
        let mut found_sector: Option<usize> = None;
        for (i, sector) in global.archery_sectors.iter().enumerate() {
            if !sector.is_full() && sector.is_inside(&primary_pos, primary_pos.level) {
                found_sector = Some(i);
                break;
            }
        }
        let sector_idx = match found_sector {
            Some(i) => i,
            None => return false,
        };

        // (2) Find nearest entry point and nearest free shooting point
        let my_sector = ctx.position.sector;

        let mut nearest_entry: Option<(usize, f32)> = None; // (index, sq_dist)
        let mut nearest_shooting: Option<(usize, f32)> = None;

        let sector = &global.archery_sectors[sector_idx];
        let primary_handle = self.base.primary_target;
        for (i, pt) in sector.points.iter().enumerate() {
            // Probe each path point with the full
            // Archer/enemy proximity predicate (per-enemy, sector- and
            // action-state-dependent threshold) — if the path passes
            // dangerously close to the primary target, abandon the
            // whole search.
            if self.archer_is_too_near_to_enemy(&pt.position, primary_handle, ctx, tick) {
                return false;
            }

            let d_to_me = pt.position.map_point() - ctx.position.map_point();
            let mut sq_dist = d_to_me.square_norm();
            // Penalty for sector changes.
            let pt_sector =
                crate::position_interface::SectorHandle::new(u16::from(pt.sector_index));
            if pt_sector != my_sector {
                sq_dist += 10000.0;
            }

            if !pt.is_shooting_point {
                if nearest_entry.is_none_or(|(_, best)| sq_dist < best) {
                    nearest_entry = Some((i, sq_dist));
                }
            } else if pt.owner.is_none() && nearest_shooting.is_none_or(|(_, best)| sq_dist < best)
            {
                nearest_shooting = Some((i, sq_dist));
            }
        }

        let (shooting_idx, _) = match nearest_shooting {
            Some(v) => v,
            None => return false, // no free shooting point
        };

        // (3) Set up archery path variables
        self.my_archery_sector_index = sector_idx as u16;
        // Fall back to the original sentinels when no shooting point
        // range was recorded, preserving the "always near head"
        // behavior in that degenerate case.
        let first_sp = sector
            .index_first_shooting_point
            .map_or(u16::MAX, u16::from);
        let last_sp = sector.index_last_shooting_point.map_or(0, u16::from);

        if let Some((entry_idx, _)) = nearest_entry {
            if (entry_idx as u16) < first_sp {
                // Near the head — run forward
                self.my_archery_point_index = crate::sector::ArcheryPointIdx(entry_idx as u16);
                self.my_archery_point_increment = 1;
            } else if (entry_idx as u16) > last_sp {
                // Near the tail — run backward
                self.my_archery_point_index = crate::sector::ArcheryPointIdx(entry_idx as u16);
                self.my_archery_point_increment = -1;
            } else {
                // Between head and tail — run directly toward shooting point
                if entry_idx < shooting_idx {
                    self.my_archery_point_index =
                        crate::sector::ArcheryPointIdx(shooting_idx.saturating_sub(1) as u16);
                    self.my_archery_point_increment = 1;
                } else {
                    self.my_archery_point_index = crate::sector::ArcheryPointIdx(
                        (shooting_idx + 1).min(sector.points.len() - 1) as u16,
                    );
                    self.my_archery_point_increment = -1;
                }
                // Already reserve this shooting point.
                self.set_my_shooting_point(global, Some((sector_idx as u16, shooting_idx as u16)));
            }
        } else {
            // No entry point — go directly to shooting point
            self.my_archery_point_index = crate::sector::ArcheryPointIdx(shooting_idx as u16);
            self.my_archery_point_increment = 1;
            self.set_my_shooting_point(global, Some((sector_idx as u16, shooting_idx as u16)));
        }

        self.set_my_archery_sector(global, Some(sector_idx as u16));
        true
    }

    /// Set the shooting point. Three-step contract: (1) clear `owner` on the
    /// previously held
    /// shooting point, (2) overwrite `my_shooting_point`, (3) write
    /// `owner` on the new shooting point.  `new` is `(sector_idx,
    /// point_idx)` into `AiGlobalState::archery_sectors`.  The
    /// sector-level `num_owners` counter is independent and is managed
    /// by `set_my_archery_sector`.
    pub(super) fn set_my_shooting_point(
        &mut self,
        global: &mut AiGlobalState,
        new: Option<(u16, u16)>,
    ) {
        if let Some((old_sec, old_pt)) = self.my_shooting_point
            && let Some(sector) = global.archery_sectors.get_mut(old_sec as usize)
            && let Some(pt) = sector.points.get_mut(old_pt as usize)
        {
            pt.owner = None;
        }
        self.my_shooting_point = new;
        if let Some((new_sec, new_pt)) = new
            && let Some(sector) = global.archery_sectors.get_mut(new_sec as usize)
            && let Some(pt) = sector.points.get_mut(new_pt as usize)
        {
            pt.owner = Some(crate::entity_id::EntityId::Soldier(
                crate::entity_id::SoldierId(self.base.me),
            ));
        }
    }

    /// Set the archery sector. Updates `my_archery_sector` and keeps the
    /// owner counter on the
    /// old/new archery sector in sync. Counter drives `is_full`, which
    /// gates sector selection in `choose_good_shooting_point`.
    fn set_my_archery_sector(&mut self, global: &mut AiGlobalState, new_sector: Option<u16>) {
        if let Some(old) = self.my_archery_sector
            && let Some(sector) = global.archery_sectors.get_mut(old as usize)
        {
            sector.decrement_owner_counter();
        }
        self.my_archery_sector = new_sector;
        if let Some(new) = new_sector
            && let Some(sector) = global.archery_sectors.get_mut(new as usize)
        {
            sector.increment_owner_counter();
        }
    }

    /// Pure read: returns the current archery waypoint
    /// on the archery path, or
    /// `None` if the cursor is past either end.  The caller is
    /// responsible for advancing via `archery_path_increment_waypoint`.
    pub(super) fn archery_path_get_waypoint(&self, global: &AiGlobalState) -> Option<PointArchery> {
        let sector = global
            .archery_sectors
            .get(self.my_archery_sector? as usize)?;
        let idx = usize::from(self.my_archery_point_index);
        sector.points.get(idx).cloned()
    }

    /// Advance the archery waypoint:
    /// `my_archery_point_index += my_archery_point_increment;` with
    /// 16-bit wrapping on overflow or underflow. After stepping off the end
    /// in either direction, the next `archery_path_get_waypoint` will
    /// see an out-of-bounds index and return `None`, matching the
    /// original game's absent-value check.
    pub(super) fn archery_path_increment_waypoint(&mut self) {
        let cur = u16::from(self.my_archery_point_index);
        let inc = i16::from(self.my_archery_point_increment);
        self.my_archery_point_index = crate::sector::ArcheryPointIdx(cur.wrapping_add_signed(inc));
    }

    /// Update the shield bearer ahead. Updates the archer's own
    /// `shield_bearer_before_me` link and the shield bearer's reciprocal
    /// `archer_behind_me` link.
    pub(super) fn update_shield_bearer_before_me(&mut self, new_sb: Option<AiEntityHandle>) {
        if !self.is_archer() {
            return;
        }
        if new_sb == self.shield_bearer_before_me {
            return;
        }
        let old_sb = self.shield_bearer_before_me;
        if let Some(old_sb) = old_sb {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetArcherBehindMe {
                    target: old_sb.get(),
                    archer: None,
                });
        }
        self.shield_bearer_before_me = new_sb;
        if let Some(new_sb) = new_sb {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SetArcherBehindMe {
                    target: new_sb.get(),
                    archer: Some(AiEntityHandle::new(self.base.me)),
                });
        }
    }

    /// Calculate the ideal position behind a linked shield bearer without
    /// testing whether the archer can move there.
    ///
    /// Original's already-in-cover check performs only this position
    /// calculation. Positioning behind the shield bearer adds the movement
    /// authorization check, but battle planning calls that method only when
    /// the archer actually needs to reposition.
    pub(super) fn shield_bearer_cover_position(
        &self,
        shield_bearer: impl IntoOptionalAiHandle,
        tick: &AiPerTickData,
    ) -> Option<Position> {
        let snap = self.find_fighter(shield_bearer, tick)?;
        // Read the bearer's "shield bearer position" — when running to
        // a phalanx slot, that's the future seek pose; once in position,
        // the current pose.
        let shield_running = Substate::AttackingRunningToPhalanx;
        let (bearer_pos, bearer_dir) = if snap.current_substate == shield_running {
            (
                snap.shield_bearer_seek_position,
                snap.shield_bearer_direction,
            )
        } else {
            (snap.position, snap.direction)
        };
        let forward = MapVec::from_sector(bearer_dir);
        let distance = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
        // Original first authors the aspect-corrected vector through
        // aspect-corrected direction-sector assignment and only then applies
        // `operator*=(DISTANCE_SHIELD_BEARER_ARCHER)`. Keep those two f32
        // roundings in that order: reassociating this as
        // `(forward.y * distance) * ASPECT_RATIO` changes the cover point by
        // one ULP for diagonal sectors.
        let vertical_offset = (forward.y * ASPECT_RATIO) * distance;
        Some(Position {
            x: bearer_pos.x - forward.x * distance,
            y: bearer_pos.y - vertical_offset,
            ..bearer_pos
        })
    }

    /// Compute the position behind the shield bearer. Given an archer caller with
    /// a linked shield bearer, compute the cover position
    /// `DISTANCE_SHIELD_BEARER_ARCHER` behind that shield bearer along
    /// their facing.
    ///
    /// Called from the `CoverBehindShieldBearer` decision after the
    /// unchecked already-in-cover calculation has established that the
    /// archer really needs to move.
    ///
    /// When the shield bearer is `AttackingRunningToPhalanx`, projects
    /// the cover point behind their *future* slot (seek position +
    /// shield-bearer direction) rather than their current pose, matching
    /// the shield-bearer positioning behavior. Returns `None` if the
    /// cover line crosses geometry (straight-movement authorization
    /// failure).
    pub(crate) fn compute_position_behind_shield_bearer(
        &self,
        shield_bearer: HumanHandle,
        env: ThinkEnv<'_>,
    ) -> Option<Position> {
        let ThinkEnv {
            ctx, tick, grid, ..
        } = env;
        let snap = self.find_fighter(shield_bearer, tick)?;
        let bearer_pos = if snap.current_substate == Substate::AttackingRunningToPhalanx {
            snap.shield_bearer_seek_position
        } else {
            snap.position
        };
        let behind = self.shield_bearer_cover_position(shield_bearer, tick)?;
        // Cover line must be unobstructed from the bearer.
        if let Some(g) = grid {
            let bearer_pt = crate::coordinates::MapPoint::new(bearer_pos.x, bearer_pos.y);
            let cover_pt = crate::coordinates::MapPoint::new(behind.x, behind.y);
            let ok =
                g.is_straight_movement_authorized(bearer_pt, cover_pt, behind.level, &ctx.move_box);
            if crate::ai_enemy::battle_decision_debug_enabled() {
                crate::ai_enemy::parity_trace::CoverPos {
                    frame: &(ctx.frame),
                    me: &(self.base.me),
                    bearer: &(shield_bearer),
                    sub: &(snap.current_substate as u32),
                    bearer_pos: &(bearer_pos),
                    bearer_dir: &(snap.shield_bearer_direction),
                    bearer_raw: &(snap.position),
                    behind: &(behind),
                    ok: &(ok),
                }
                .emit();
            }
            if !ok {
                return None;
            }
        }
        Some(behind)
    }

    /// Propose combat positions.
    // -----------------------------------------------------------------------
    // Reconsider the swordfight
    // -----------------------------------------------------------------------

    pub(crate) fn reconsider_swordfight(
        &mut self,
        _env: ThinkEnv<'_>,
        enemy_weak: bool,
        _global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::ReconsiderSwordfight { enemy_weak },
            after: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------
    // Choose a step-back goal
    // -----------------------------------------------------------------------

    /// Compute a retreat position away from `pos_enemy`.
    /// Delegates to the free function [`propose_good_step_back_goal`].
    pub(crate) fn propose_good_step_back_goal(
        &self,
        pos_enemy: Position,
        good_distance: u16,
        min_distance: u16,
        env: ThinkEnv<'_>,
        aspect_ratio: f32,
    ) -> Option<Position> {
        propose_good_step_back_goal(
            env.ctx.position,
            &env.ctx.move_box,
            pos_enemy,
            good_distance,
            min_distance,
            env.grid,
            aspect_ratio,
        )
    }

    // -----------------------------------------------------------------------
    // Reconsider swordfight observation
    // -----------------------------------------------------------------------

    /// EVENT_TIMER handler for `Substate::AttackingObserve`. Runs its
    /// own decision body literally rather than dispatching through
    /// `battle_decisions` (which has a different decision tree). Walks
    /// these steps:
    ///   1. arrow-protection refresh guard
    ///   2. rebuild list_them with combat readiness, maximum-axis distance below
    ///      MAX_SWORDFIGHT_CONSIDERATION_RADIUS, and forward-half-plane detection
    ///   3. rebuild list_us and bump local primary-target multiplicity for
    ///      same-camp soldiers in any swordfight substate
    ///   4. select a new primary target, strongly preferring unoccupied targets
    ///      with the local multiplicity override
    ///   5. Focus(primary)
    ///   6. null primary → evaluate the battle overview and bail
    ///   7. combat_trainer → set direction, observe, start a 20-tick timer, and bail
    ///   8. defensive predecision → step-back goal or directed panic
    ///   9. attack-opportunity block (back-to-me / not-swordfighting /
    ///      principal opponent dogpiled / very close) gated on no friend
    ///      already approaching the same target
    ///   10. fall through to `observe_and_step` for repositioning
    pub(super) fn reconsider_swordfight_observation(
        &mut self,
        _env: ThinkEnv<'_>,
        _global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::ReconsiderSwordfightObservation,
            after: Vec::new(),
        })
    }
}

pub(crate) fn drunk_combat_freezes(sim: &SimulationContext, blood_alcohol: u8) -> bool {
    crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
        <= blood_alcohol as u16
        || crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
            <= blood_alcohol as u16
}

#[cfg(test)]
mod tests;
