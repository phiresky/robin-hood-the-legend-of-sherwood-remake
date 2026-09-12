use super::*;

#[test]
fn broad_swordfight_family_includes_approach_and_special_strike() {
    assert!(crate::ai::Substate::AttackingRunningToEnemy.is_any_swordfight());
    assert!(crate::ai::Substate::AttackingSwordfightSpecialStrike.is_any_swordfight());
    assert!(!crate::ai::Substate::AttackingTooProudToAttack.is_any_swordfight());
}
