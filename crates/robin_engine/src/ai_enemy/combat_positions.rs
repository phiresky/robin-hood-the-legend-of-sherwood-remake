//! Combat positions, phalanx/shield-bearer formation, archery
//! shooting-point selection, and the swordfight repositioning loop.
//!
//! Owns the helpers used by `propose_good_combat_position`,
//! `reconsider_swordfight` and `reconsider_swordfight_observation`.
//! Also exposes `find_fighter`,
//! `is_allowed_to_attack`, and the neighbour predicates.

use crate::ai::*;
use crate::sim_rng::SimulationContext;

use super::EnemyAi;
use super::util::vec_to_sector;
use crate::coordinates::MapVec;

/// Us / them aggregates built by `reconsider_swordfight`.
#[derive(Clone, Copy)]
pub(crate) struct SwordfightLists {
    pub(crate) nearest_friend_solo: Option<AiEntityHandle>,
    pub(crate) number_of_swordfighting_enemies: u16,
    pub(crate) number_of_friends: u16,
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
}

pub(crate) fn drunk_combat_freezes(sim: &SimulationContext, blood_alcohol: u8) -> bool {
    crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
        <= blood_alcohol as u16
        || crate::sim_rng::u16(sim, crate::sim_rng::RngSite::DrunkCombatFreeze, 0..100)
            <= blood_alcohol as u16
}

#[cfg(test)]
mod tests;
