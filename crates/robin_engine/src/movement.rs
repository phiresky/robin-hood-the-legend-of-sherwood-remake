//! Movement ability kinds and conversion of pathfinder waypoints into sequence orders.
//!
//! Actors execute these orders through the sprite motion pipeline.

use crate::coordinates::MapPoint;
use crate::order::{Order, OrderType};
use crate::sequence::SequenceElement;

// ═══════════════════════════════════════════════════════════════════
//  Ability kinds
// ═══════════════════════════════════════════════════════════════════

/// Which hero ability is currently being performed.
///
/// Each variant maps to a specific animation and state transition —
/// see [`crate::abilities`] for the full dispatch logic.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AbilityKind {
    /// Little John picks up an unconscious/dead body.
    Carry,
    /// Little John drops a carried body.
    Drop,
    /// Any PC ties up an unconscious enemy.
    Tie,
    /// Friar Tuck heals a wounded PC.
    Heal,
    /// Robin Hood whistles to attract guards.
    Whistle,
    /// Any PC listens for nearby blipped NPCs / objects / FX targets.
    /// Drives a fixed-length countdown (`TIME_LISTEN_WAIT` = 25 frames)
    /// in the selected PC owner arm, which fires a one-shot reveal + FX-target
    /// `Heard()` callback when it reaches 0.
    Listen,
    /// Stuteley throws a net trap.
    ThrowNet,
    /// Stuteley throws a wasp nest.
    ThrowWaspNest,
    /// Any PC throws a coin purse — bursts into coins on impact and
    /// distracts nearby soldiers.  Drives the `ThrowingPurse` animation;
    /// on completion spawns the purse projectile, whose impact handler
    /// in `engine::purse` ejects child coins.
    ThrowPurse,
    /// Little John or another PC throws an apple at a soldier or FX
    /// target.
    ThrowApple,
    /// PC throws a stone at a soldier or FX target.
    ThrowStone,
    /// A VIP PC pays a beggar civilian.  Drives the `Paying` animation;
    /// on completion subtracts [`BEGGAR_SALARY`] from the ransom and
    /// spawns a [`Command::ReceivePurse`] sequence element on the beggar.
    ///
    /// [`BEGGAR_SALARY`]: crate::engine::BEGGAR_SALARY
    /// [`Command::ReceivePurse`]: crate::element::Command::ReceivePurse
    Pay,
    /// A beggar civilian plays the three-animation purse response:
    /// `ReceivingPurse` → `WaitingWithPurse` → transition back.  When
    /// the middle animation completes, [`EngineInner::reveal_scrolls`] runs
    /// and queues up the next scroll set.
    ///
    /// [`EngineInner::reveal_scrolls`]: crate::engine::EngineInner::reveal_scrolls
    ReceivePurse,
    /// A PC punches a human target.  Drives the `Hitting` animation;
    /// on the "done" frame launches a [`Command::ReceiveHitDamage`] damage
    /// element with concussion 80 (regular hit) or 150 (hard hit)
    /// depending on whether the attacker's profile carries the HIT_HARD
    /// action slot.
    ///
    /// [`Command::ReceiveHitDamage`]: crate::element::Command::ReceiveHitDamage
    Hit,
    /// A PC strangles an NPC.  Drives the `Strangling` animation;
    /// on completion launches a [`Command::ReceiveDamage`] element that
    /// zeroes the victim's life points (unless the soldier is not
    /// stranglable, in which case the animation simply ends and the
    /// soldier retaliates).
    ///
    /// [`Command::ReceiveDamage`]: crate::element::Command::ReceiveDamage
    Strangle,
    /// A PC eats a ration to recover life points.  Drives the
    /// `Eating` animation; on the "done" frame decrements the ration ammo
    /// counter (Eat and Guzzle share the same `num_rations` counter)
    /// and adds 40 (Eat) or 80 (Guzzle) life points, capped at
    /// `LIFEPOINTS_PC`.
    Eat,
    /// A PC climbs onto a HelpingToClimb partner's shoulders.  Drives the
    /// `ClimbingUpOnShoulders` animation on the climber while the helper
    /// (carrier) plays a sync'd `TransitionHelpingClimbingUp`.  On the
    /// "done" frame both PCs settle into the paired `OnShoulders` /
    /// `CarryingOnShoulders` postures.
    ClimbOnShoulders,
    /// A PC dismounts from its `HelpingToClimb` carrier.  Drives the
    /// `ClimbingDownFromShoulders` animation on the climber while the
    /// helper plays a sync'd `TransitionHelpingClimbingDown`.  On the
    /// "terminated" frame both PCs settle back into `Upright` /
    /// `HelpingToClimb` postures and the carrier link is severed.
    ClimbDownFromShoulders,
    /// A PC with the Tie contextual action releases a tied NPC.
    /// Drives the existing `Tying` animation in reverse.
    Untie,
}

impl AbilityKind {
    pub const ALL: [Self; 19] = [
        Self::Carry,
        Self::Drop,
        Self::Tie,
        Self::Heal,
        Self::Whistle,
        Self::Listen,
        Self::ThrowNet,
        Self::ThrowWaspNest,
        Self::ThrowPurse,
        Self::ThrowApple,
        Self::ThrowStone,
        Self::Pay,
        Self::ReceivePurse,
        Self::Hit,
        Self::Strangle,
        Self::Eat,
        Self::ClimbOnShoulders,
        Self::ClimbDownFromShoulders,
        Self::Untie,
    ];
}

// ═══════════════════════════════════════════════════════════════════
//  Order building from pathfinder waypoints
// ═══════════════════════════════════════════════════════════════════

/// Convert pathfinder waypoints into movement orders on a sequence element.
///
/// Each waypoint becomes an [`Order`] with the given `action` animation.
/// Intermediate waypoints get tolerance 0 (pass through exactly);
/// the final waypoint gets the requested `tolerance`.  `reverse` is
/// stamped on every order.  `antagonist`, when `Some`, rides on the final
/// order only — the "apply tolerance & antagonist on last order" pattern.
pub fn build_orders_from_path(
    element: &mut SequenceElement,
    waypoints: &[MapPoint],
    action: OrderType,
    tolerance: f32,
    reverse: bool,
    antagonist: Option<crate::element::EntityId>,
    next_order_id: &mut u32,
    reuse_restored_waiting_tail: bool,
) {
    if !reuse_restored_waiting_tail {
        let transition_orders = element.num_transition_orders.min(element.orders.len());
        element.orders.truncate(transition_orders);
        element.num_transition_orders = transition_orders;
    }
    let mut first = true;
    let last = waypoints.len().saturating_sub(1);
    for (i, &wp) in waypoints.iter().enumerate() {
        let mut order = Order::new(
            action,
            wp.x,
            wp.y,
            crate::order::alloc_order_id(next_order_id),
        );
        if matches!(
            action,
            OrderType::ClimbingWallUp
                | OrderType::ClimbingWallDown
                | OrderType::ClimbingWallUpFast
                | OrderType::ClimbingWallDownFast
                | OrderType::ClimbingLadderUp
                | OrderType::ClimbingLadderDown
                | OrderType::ClimbingLadderUpFast
                | OrderType::ClimbingLadderDownFast
        ) {
            order.compute_direction = false;
        }
        order.reverse = reverse;
        // Only the final waypoint gets the requested tolerance and
        // antagonist.  For a single-waypoint direct path we still stamp
        // both since the final/only order is also the last.
        if i == last {
            order.tolerance = tolerance;
            order.antagonist = antagonist;
        } else {
            order.tolerance = 0.0;
        }

        if reuse_restored_waiting_tail
            && first
            && let Some(existing) = element.orders.back_mut()
        {
            // Original-game path processing reuses the movement element's
            // last pre-path order: assign a new ID, change its action/destination,
            // and leave every preceding order in place. In particular, a
            // loaded MOVE_WAITING can still own a start transition followed
            // by a freezing order. Discarding that prefix makes the actor
            // begin moving one frame earlier than the saved Original.
            //
            // Preserve the old order's otherwise-authoritative fields just
            // like the original game's in-place mutation, while retaining Rust's typed
            // representation for climb direction handling.
            existing.order_id = order.order_id;
            existing.order_type = order.order_type;
            existing.target_x = order.target_x;
            existing.target_y = order.target_y;
            existing.reverse = order.reverse;
            if !order.compute_direction {
                existing.compute_direction = false;
            }
            if i == last {
                existing.tolerance = order.tolerance;
                existing.antagonist = order.antagonist;
            }
        } else {
            element.push_order(order);
        }
        first = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;

    #[test]
    fn build_orders_from_path_sets_tolerance() {
        let mut elem = SequenceElement::new(
            1,
            crate::element::Command::Move,
            Some(crate::element::EntityId::Pc(crate::entity_id::PcId(0))),
        );
        let waypoints = vec![map_pt(10.0, 20.0), map_pt(30.0, 40.0), map_pt(50.0, 60.0)];

        let mut next_order_id = 1u32;
        build_orders_from_path(
            &mut elem,
            &waypoints,
            OrderType::WalkingUpright,
            5.0,
            false,
            None,
            &mut next_order_id,
            false,
        );

        assert_eq!(elem.orders.len(), 3);
        // Intermediate waypoints have tolerance 0
        assert_eq!(elem.orders[0].tolerance, 0.0);
        assert_eq!(elem.orders[1].tolerance, 0.0);
        // Final waypoint gets the requested tolerance
        assert_eq!(elem.orders[2].tolerance, 5.0);

        // Check coordinates
        assert_eq!(elem.orders[0].target_x, 10.0);
        assert_eq!(elem.orders[0].target_y, 20.0);
        assert_eq!(elem.orders[2].target_x, 50.0);
        assert_eq!(elem.orders[2].target_y, 60.0);
    }

    #[test]
    fn build_orders_from_path_reuses_last_wait_and_preserves_prefix() {
        let mut elem = SequenceElement::new(
            1,
            crate::element::Command::Move,
            Some(crate::element::EntityId::Pc(crate::entity_id::PcId(0))),
        );
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingCapeWaitingUpright,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(OrderType::Freezing, 0.0, 0.0));
        let transition_id = elem.orders[0].order_id;
        let wait_id = elem.orders[1].order_id;

        let mut next_order_id = 100u32;
        build_orders_from_path(
            &mut elem,
            &[map_pt(10.0, 20.0)],
            OrderType::WalkingUpright,
            0.0,
            false,
            None,
            &mut next_order_id,
            true,
        );

        assert_eq!(elem.orders.len(), 2);
        assert_eq!(elem.orders[0].order_id, transition_id);
        assert_eq!(
            elem.orders[0].order_type,
            OrderType::TransitionWaitingCapeWaitingUpright
        );
        assert_eq!(elem.orders[1].order_type, OrderType::WalkingUpright);
        assert_ne!(elem.orders[1].order_id, wait_id);
        assert_eq!(elem.orders[1].order_id.get(), 100);
        assert_eq!(elem.orders[1].target_x, 10.0);
        assert_eq!(elem.orders[1].target_y, 20.0);
    }
}
