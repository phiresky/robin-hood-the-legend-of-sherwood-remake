//! Human combat-movement action and directional-animation selection.
//!
//! These helpers mirror `RHElementActorHuman::DetermineMovementAnimation`,
//! `FaceOpponent`, and `FaceDangerPoint` without owning movement state.

use super::{OrderType, door_pass_sprite_animation_override};

pub(super) fn is_sword_movement_nonanimation(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingWithSword | OrderType::RunningWithSword
    )
}

pub(super) fn is_sword_motion_context(
    action_state: crate::element::ActionState,
    door_pass_action: Option<OrderType>,
    order_action: OrderType,
) -> bool {
    let selected_action =
        door_pass_sprite_animation_override(order_action, door_pass_action).unwrap_or(order_action);
    let stale_sword_state = matches!(
        action_state,
        crate::element::ActionState::MovingSword | crate::element::ActionState::MovingFastSword
    );

    // Human's FaceOpponent override owns only its literal sword-movement
    // Execute arms. DetermineMovementAnimation performs the state-dependent
    // WALKING_UPRIGHT -> WALKING_WITH_SWORD rewrite while translating an
    // ordinary movement element (`RHelementactorhuman.cpp:2365-2401`). A
    // PassDoor step is already a concrete order selected by the door
    // translator; Execute dispatches that literal action. Thus neither an
    // ordinary WalkingUpright door step nor a wall/ladder transition may be
    // reclassified from the preceding step's still-latched MOVING_SWORD
    // state. Only non-door ordinary movement retains that translation-time
    // fallback in Rust, where the successor START effect has not yet
    // published its new state.
    let stale_state_converts_selected_action = door_pass_action.is_none()
        && matches!(
            selected_action,
            OrderType::WalkingUpright | OrderType::RunningUpright | OrderType::WalkingWithCorpse
        );
    is_sword_movement_nonanimation(selected_action)
        || stale_sword_state && stale_state_converts_selected_action
}

/// Whether the selected logical action enters Human's sword-movement Execute
/// arm. The broader sword-motion context also retains the outgoing live state
/// for facing and sprite selection, but that stale state does not own START
/// side effects once an ordinary successor is selected.
pub(super) fn executes_sword_movement_action(
    door_pass_action: Option<OrderType>,
    order_action: OrderType,
) -> bool {
    let action =
        door_pass_sprite_animation_override(order_action, door_pass_action).unwrap_or(order_action);
    is_sword_movement_nonanimation(action)
}

/// Match `RHElementActorHuman::DetermineMovementAnimation`: these are logical
/// dispatch tokens consumed by the Human Execute override, not sprite rows.
/// `FaceOpponent` chooses the concrete forward/backward/strafe sword animation
/// later, so sprite animation availability must not gate this rewrite.
pub(super) fn sword_movement_dispatch_action(order: OrderType) -> OrderType {
    match order {
        OrderType::WalkingUpright | OrderType::WalkingWithCorpse => OrderType::WalkingWithSword,
        OrderType::RunningUpright => OrderType::RunningWithSword,
        _ => order,
    }
}

/// Signed angle from the movement displacement to the actor's facing vector,
/// as `RHElementActorHuman::FaceOpponent` measures it, normalised to
/// `[0, 2π)`.
///
/// This reproduces the determinant/dot form rather than differencing two
/// `atan2` results, because the two disagree once the determinant vanishes.
/// A degenerate displacement — an order whose destination is already the
/// actor's position — makes both terms zero and yields a half turn, so the
/// actor walks backwards rather than inheriting whatever direction it happens
/// to be facing.
pub(super) fn combat_movement_angle(displacement: (f32, f32), facing: (f32, f32)) -> f32 {
    let (dx, dy) = displacement;
    let (fx, fy) = facing;
    let dot = dx * fx + dy * fy;
    let det = dx * fy - dy * fx;
    let mut angle = if det == 0.0 {
        if dot > 0.0 { 0.0 } else { std::f32::consts::PI }
    } else {
        // The ratio is formed in single precision before the arc tangent,
        // so an overflow to infinity here still resolves to a quarter turn.
        let raw = f64::from(det / dot).atan() as f32;
        if dot >= 0.0 {
            raw
        } else if det > 0.0 {
            raw + std::f32::consts::PI
        } else {
            raw - std::f32::consts::PI
        }
    };
    if angle < 0.0 {
        angle += 2.0 * std::f32::consts::PI;
    }
    if angle >= 2.0 * std::f32::consts::PI {
        angle -= 2.0 * std::f32::consts::PI;
    }
    angle
}

pub(super) fn combat_directional_animation(
    action_state: crate::element::ActionState,
    angle: f32,
) -> OrderType {
    let unit = std::f32::consts::FRAC_PI_4;
    match action_state {
        crate::element::ActionState::MovingShield => {
            // FaceDangerPoint's four comparisons all reject NaN, then its
            // release-build fallback returns WALKING_SHIELD.
            if angle.is_nan() {
                return OrderType::WalkingShield;
            }
            if angle < unit || angle >= 7.0 * unit {
                OrderType::WalkingShield
            } else if angle < 3.0 * unit {
                OrderType::StrafingRightShield
            } else if angle < 5.0 * unit {
                OrderType::WalkingBackwardsShield
            } else {
                OrderType::StrafingLeftShield
            }
        }
        _ => {
            // FaceOpponent likewise falls through its disabled assertion to
            // WALKING_SWORD when unchecked normalization produced a NaN goal.
            if angle.is_nan() {
                return OrderType::WalkingSword;
            }
            if angle < unit || angle >= 7.0 * unit {
                OrderType::WalkingSword
            } else if angle < 3.0 * unit {
                OrderType::StrafingRightSword
            } else if angle < 5.0 * unit {
                OrderType::WalkingBackwardsSword
            } else {
                OrderType::StrafingLeftSword
            }
        }
    }
}
