use super::*;

// ─── Animation selection ────────────────────────────────────────────

/// Animation category for combat state transitions.
///
/// Used by the damage-translation paths (sword / push / hit / arrow).
#[derive(Debug, Clone, Copy)]
pub(super) struct CombatAnimations {
    pub(super) falling_back: crate::order::OrderType,
    pub(super) dying_forward: crate::order::OrderType,
    /// Used by stand-up-after-push sequences (TranslatePushDamage).
    pub(super) standing_up: crate::order::OrderType,
    /// Used by non-KO hit reactions (TranslateSwordDamage simple hit path).
    pub(super) simple_hit: crate::order::OrderType,
    /// Survivor animation for arrow / piercing hits.
    /// `ExtractingArrow{Upright,Crouched,Sword,Bow}` per the
    /// posture/action switch.
    pub(super) arrow_extract: crate::order::OrderType,
}

/// Select combat animations based on current posture and action state.
pub(super) fn select_combat_animations(
    posture: Posture,
    action_state: ActionState,
) -> Option<CombatAnimations> {
    use crate::order::OrderType;
    match posture {
        // `Undefined` is treated as `Upright` everywhere else (see
        // sprite-row selection in `element.rs`).  Without it in this
        // arm, NPCs that still carry the default load-time posture
        // (soldiers never explicitly set it) get no falling / push /
        // hit animation at KO time.
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackSword,
                    dying_forward: OrderType::DyingSword,
                    standing_up: OrderType::StandingUpSword,
                    simple_hit: OrderType::BeingHitSword,
                    arrow_extract: OrderType::ExtractingArrowSword,
                })
            } else if action_state.is_bow() {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackBow,
                    dying_forward: OrderType::DyingBow,
                    standing_up: OrderType::StandingUpBow,
                    simple_hit: OrderType::FallingBackBow,
                    arrow_extract: OrderType::ExtractingArrowBow,
                })
            } else {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackUpright,
                    dying_forward: OrderType::DyingUpright,
                    standing_up: OrderType::StandingUp,
                    simple_hit: OrderType::FallingBackUpright,
                    arrow_extract: OrderType::ExtractingArrowUpright,
                })
            }
        }
        Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree => Some(CombatAnimations {
            falling_back: OrderType::FallingBackCrouched,
            dying_forward: OrderType::DyingCrouched,
            standing_up: OrderType::StandingUp,
            simple_hit: OrderType::FallingBackCrouched,
            arrow_extract: OrderType::ExtractingArrowCrouched,
        }),
        // Already lying / dead / carried — no animation needed
        _ => None,
    }
}

/// Push-damage animation set, selected based on posture and action state.
#[derive(Debug, Clone, Copy)]
pub(super) struct PushDamageAnimations {
    /// The falling-pushed animation to play.
    pub(super) falling: crate::order::OrderType,
    /// Standing-up animation.  Crouched-family pushes deliberately retain
    /// Original's `NonanimationEnd` sentinel here; see the selector below.
    pub(super) standing_up: Option<crate::order::OrderType>,
    /// Stunned animation if concussion > threshold (None if not applicable).
    pub(super) stunned: Option<crate::order::OrderType>,
}

/// Select push-damage animations based on posture and action state.
///
/// Returns `None` for postures that don't get a falling animation
/// (already lying, dead, carried, on ladder/wall, etc.).
pub(super) fn select_push_damage_animations(
    posture: Posture,
    action_state: ActionState,
) -> Option<PushDamageAnimations> {
    use crate::order::OrderType;
    match posture {
        // `Undefined` is treated as `Upright` everywhere else (see
        // sprite-row selection in `element.rs`).  Without it in this
        // arm, NPCs that still carry the default load-time posture
        // (soldiers never explicitly set it) get no falling / push /
        // hit animation at KO time.
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedWithSword,
                    standing_up: Some(OrderType::StandingUpSword),
                    stunned: Some(OrderType::BeingStunnedSword),
                })
            } else if action_state.is_bow() {
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedWithBow,
                    standing_up: Some(OrderType::StandingUpBow),
                    stunned: None,
                })
            } else {
                // Waiting, bored, moving, shield, sleeping, listening.
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedUpright,
                    standing_up: Some(OrderType::StandingUp),
                    stunned: None,
                })
            }
        }
        Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree => {
            Some(PushDamageAnimations {
                falling: OrderType::FallingPushedCrouched,
                // Original initializes `standupAnimation` to
                // RHNONANIMATION_END, changes only `fallingAnimation` in
                // this posture arm, and nevertheless appends the stand-up
                // order for a conscious stunning hit.  That sentinel is a
                // real one-slot order: it keeps ReceiveSwordDamage selected
                // for one more Hourglass before a postponed successor runs.
                // See original-code/RHelementactorhuman.cpp
                // TranslatePushDamage.
                standing_up: Some(OrderType::NonanimationEnd),
                stunned: None,
            })
        }
        Posture::OnLadder | Posture::OnWall => {
            // `translate_ladder_wall_fall` handles this case.  The
            // caller's `apply_push_effect` detects OnLadder/OnWall
            // before calling this function and branches into that
            // helper instead; returning an animation here would double
            // up the work.
            None
        }
        // Already lying, dead, carried, tied, flying: no animation
        _ => None,
    }
}

/// Select the falling-hit animation for `TranslateHitDamage`.
///
/// Returns `None` for postures treated as already-falling / dead /
/// carried (LYING, FLYING, CARRIED, ON_SHOULDERS, TIED, DEAD,
/// DEAD_BACK, STUCK_UNDER_NET).
///
/// When `harder` is true, the `HARDER` variant is returned. The harder
/// variant plays in place and collapses to `Lying` at the end, while
/// the non-harder variant flights 30 units away from the attacker.
pub(in crate::engine) fn select_hit_fall_animation(
    posture: Posture,
    action_state: ActionState,
    harder: bool,
) -> Option<crate::order::OrderType> {
    use crate::order::OrderType;
    match posture {
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_bow() {
                Some(if harder {
                    OrderType::FallingHitHarderWithBow
                } else {
                    OrderType::FallingHitWithBow
                })
            } else if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(if harder {
                    OrderType::FallingHitHarderWithSword
                } else {
                    OrderType::FallingHitWithSword
                })
            } else {
                Some(if harder {
                    OrderType::FallingHitHarderUpright
                } else {
                    OrderType::FallingHitUpright
                })
            }
        }
        Posture::Crouched | Posture::Tree | Posture::SimulatingBeggar => Some(if harder {
            OrderType::FallingHitHarderCrouched
        } else {
            OrderType::FallingHitCrouched
        }),
        // TranslateHitDamage just terminates for these postures —
        // no animation needed.
        _ => None,
    }
}
