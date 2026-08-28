//! Per-NPC `DetectableType::Enemy` filter.
//!
//! Rejects the AddDetectable push when the (NPC camp, NPC kind, target
//! kind, target camp) combination doesn't match one of the four accepted
//! arms.  The fan-out broadcaster
//! `engine/reinforcement.rs::add_detectable_for_all_npc` and the AI
//! drain in `engine/ai/mod.rs` both consult this helper.

use crate::element_kinds::Camp;

/// `true` when an NPC of the given camp/role should accept `target` as
/// a `DETECTABLE_ENEMY` entry.
///
/// - Royalist soldier: target must be a Lacklandist soldier.
/// - Royalist civilian: target must be a PC.
/// - Lacklandist soldier: target must be a Royalist soldier OR a PC.
/// - Lacklandist civilian: target must be a PC.
pub fn should_add_enemy_detectable(
    npc_camp: Camp,
    npc_is_soldier: bool,
    target_is_pc: bool,
    target_is_soldier: bool,
    target_camp: Camp,
) -> bool {
    if npc_camp == Camp::Error || target_camp == Camp::Error {
        tracing::warn!(
            ?npc_camp,
            ?target_camp,
            npc_is_soldier,
            target_is_pc,
            target_is_soldier,
            "rejecting enemy detectable with invalid allegiance"
        );
        return false;
    }
    if !npc_is_soldier {
        // Preserve Original's unusual legacy behavior for both built-in
        // civilian camps. Custom civilians compare the target PC's authored
        // allegiance instead of inheriting Original's all-PCs-are-Royalist
        // assumption.
        return target_is_pc
            && (!matches!(npc_camp, Camp::Custom(_)) || npc_camp.is_hostile_to(target_camp));
    }
    (target_is_soldier || target_is_pc) && npc_camp.is_hostile_to(target_camp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn royalist_soldier_only_accepts_lacklandist_soldiers() {
        assert!(should_add_enemy_detectable(
            Camp::Royalists,
            true,
            false,
            true,
            Camp::Lacklandists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Royalists,
            true,
            true,
            false,
            Camp::Royalists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Royalists,
            true,
            false,
            false,
            Camp::Lacklandists
        ));
    }

    #[test]
    fn royalist_civilian_accepts_pcs_only() {
        assert!(should_add_enemy_detectable(
            Camp::Royalists,
            false,
            true,
            false,
            Camp::Royalists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Royalists,
            false,
            false,
            true,
            Camp::Lacklandists
        ));
    }

    #[test]
    fn lacklandist_soldier_accepts_royalist_soldiers_and_pcs() {
        assert!(should_add_enemy_detectable(
            Camp::Lacklandists,
            true,
            false,
            true,
            Camp::Royalists
        ));
        assert!(should_add_enemy_detectable(
            Camp::Lacklandists,
            true,
            true,
            false,
            Camp::Royalists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Lacklandists,
            true,
            false,
            true,
            Camp::Lacklandists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Lacklandists,
            true,
            false,
            false,
            Camp::Royalists
        ));
    }

    #[test]
    fn lacklandist_civilian_accepts_pcs_only() {
        assert!(should_add_enemy_detectable(
            Camp::Lacklandists,
            false,
            true,
            false,
            Camp::Royalists
        ));
        assert!(!should_add_enemy_detectable(
            Camp::Lacklandists,
            false,
            false,
            true,
            Camp::Royalists
        ));
    }

    #[test]
    fn error_camp_rejects_everything() {
        assert!(!should_add_enemy_detectable(
            Camp::Error,
            true,
            true,
            true,
            Camp::Royalists
        ));
    }

    #[test]
    fn custom_soldiers_detect_every_distinct_allegiance() {
        let observer = Camp::Custom(7);
        assert!(!should_add_enemy_detectable(
            observer,
            true,
            false,
            true,
            Camp::Custom(7),
        ));
        for id in 2..12 {
            if id != 7 {
                assert!(should_add_enemy_detectable(
                    observer,
                    true,
                    false,
                    true,
                    Camp::Custom(id),
                ));
            }
        }
    }

    #[test]
    fn custom_civilian_rejects_allied_pc_and_accepts_hostile_pc() {
        assert!(!should_add_enemy_detectable(
            Camp::Custom(7),
            false,
            true,
            false,
            Camp::Custom(7),
        ));
        assert!(should_add_enemy_detectable(
            Camp::Custom(7),
            false,
            true,
            false,
            Camp::Custom(8),
        ));
    }
}
