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

    /// Set the shooting point. Three-step contract: (1) clear `owner` on the
    /// previously held
    /// shooting point, (2) overwrite `my_shooting_point`, (3) write
    /// `owner` on the new shooting point.  `new` is `(sector_idx,
    /// point_idx)` into `AiGlobalState::archery_sectors`.  The
    /// sector-level `num_owners` counter is independent and is managed
    /// by `set_my_archery_sector`.
    pub(crate) fn set_my_shooting_point(
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
    /// gates shooting-sector selection.
    pub(crate) fn set_my_archery_sector(
        &mut self,
        global: &mut AiGlobalState,
        new_sector: Option<u16>,
    ) {
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
