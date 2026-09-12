use super::*;

#[test]
fn broad_swordfight_family_includes_approach_and_special_strike() {
    assert!(is_any_swordfight_substate(
        crate::ai::Substate::AttackingRunningToEnemy as u32
    ));
    assert!(is_any_swordfight_substate(
        crate::ai::Substate::AttackingSwordfightSpecialStrike as u32
    ));
    assert!(!is_any_swordfight_substate(
        crate::ai::Substate::AttackingTooProudToAttack as u32
    ));
}
