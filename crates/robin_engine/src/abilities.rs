//! Hero special abilities — carry, tie-up, heal, whistle, listen, trap placement.
//!
//! Each ability follows the [`crate::bow_shot`] pattern:
//!
//! 1. A `begin_*` function is called when the engine dispatches a
//!    `Command::*` sequence element to an actor.  It validates the actor
//!    and target, pushes an animation order, and sets
//!    [`ActiveAbility`][crate::movement::ActiveAbility] on the actor.
//!
//! 2. [`tick_abilities`] runs every engine tick and, for each actor with
//!    an active ability, drives the sprite animation via `perform_action`.
//!    When the sprite reports [`SpriteMotionState::Done`], the tick
//!    records the result and clears the ability state.
//!
//! 3. The engine applies cross-entity effects (posture changes, HP
//!    restoration, etc.) from the returned [`AbilityTickResult`] values
//!    *after* the mutable entity borrow is released.

use crate::element::{
    ActionState, Entity, EntityId, GameMaterial, ListenPhase, Point2D as ElemPoint2D, Posture,
    ReceivePursePhase,
};
use crate::movement::{AbilityKind, ActiveAbility};
use crate::order::{Order, OrderType};
use crate::position_interface::{ObstacleHandle, PlaneZCoeffs};
use crate::sequence::{SequenceId, SequenceManager};
use crate::sprite::MotionState as SpriteMotionState;

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

/// HP restored per bandage.
pub const HEAL_AMOUNT: i16 = 75;

/// Max life points for PCs.
pub const LIFEPOINTS_PC: i16 = 100;

/// Max distance² for healing / tying (40² = 1600).
pub const DISTANCE_MAX_SQ: f32 = 1600.0;

/// Whistle noise radius.
pub const NOISE_VOLUME_WHISTLE: u16 = 400;

/// Frames the Listen / Whistle ability stays active before its one-shot
/// effect fires (Listen reveal, Whistle ellipse fully expanded).
pub const TIME_LISTEN_WAIT: u32 = 25;

/// Final-frames window during which the expanding noise ellipse is
/// rendered for Listen/Whistle.
pub const TIME_LISTEN: u32 = 5;

/// Predicate: can a carrier currently carry another PC on their shoulders
/// without hitting a low ceiling?
///
/// Casts a vertical ray from `z + 50` up to `z + 90` at the carrier's
/// `(x, y)` and tests whether the column is free of `SIGHTOBSTACLE_SOLID`
/// obstacles.
///
/// Returns `true` when the carrier has headroom; `false` when a ceiling
/// blocks the carried body.
pub fn can_carry_on_shoulders(
    carrier_position: crate::position_interface::Point3D,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    use crate::sight_obstacle::SIGHTOBSTACLE_SOLID;
    let ground = [
        carrier_position.x,
        carrier_position.y,
        carrier_position.z + 50.0,
    ];
    let air = [
        carrier_position.x,
        carrier_position.y,
        carrier_position.z + 90.0,
    ];
    crate::sight_obstacle::is_reachable_3d(obstacles, air, ground, SIGHTOBSTACLE_SOLID)
}

/// Net apex height for trajectory.
pub const APEX_NET: f32 = 30.0;

/// Wasp nest apex height.
pub const APEX_WASP_NEST: f32 = 50.0;

// ═══════════════════════════════════════════════════════════════════
//  Order ID generator
// ═══════════════════════════════════════════════════════════════════

use std::num::NonZeroU32;

/// Allocate a fresh ability order-id.  Delegates to
/// `crate::order::alloc_order_id` so every site in the engine uses the
/// same id-allocation logic (skip-zero on wrap).
fn alloc_order_id(counter: &mut u32) -> NonZeroU32 {
    crate::order::alloc_order_id(counter)
}

/// Allocate a new ability order-id for the Listen phase machine.
///
/// Used by `engine/ai.rs` when the Listen countdown completes and
/// we need a fresh `order_id` so `perform_action` starts the exit
/// transition animation instead of continuing the previous one.
pub(crate) fn next_listen_order_id(counter: &mut u32) -> NonZeroU32 {
    alloc_order_id(counter)
}

// ═══════════════════════════════════════════════════════════════════
//  Begin result
// ═══════════════════════════════════════════════════════════════════

/// Outcome of attempting to start an ability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginResult {
    /// Ability animation started; sequence element is now `InProgress`.
    Started,
    /// Actor or target not in a valid state; mark element `Impossible`.
    Impossible,
}

// ═══════════════════════════════════════════════════════════════════
//  Tick result — returned to the engine for cross-entity effects
// ═══════════════════════════════════════════════════════════════════

/// Describes what happened when an ability animation completed.
///
/// The engine applies these effects after the mutable entity borrow
/// is released, avoiding double-borrow issues.
pub enum AbilityTickResult {
    /// Little John finished picking up a body.
    CarryDone {
        carrier_id: EntityId,
        target_id: EntityId,
        /// Posture the target had before being picked up (to restore on drop).
        carried_posture: Posture,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Little John finished dropping a body.
    DropDone {
        carrier_id: EntityId,
        target_id: EntityId,
        /// Posture to restore on the dropped body.
        drop_posture: Posture,
        /// Position to place the dropped body at.
        carrier_pos: ElemPoint2D,
        carrier_direction: u16,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// PC finished tying up an unconscious enemy.
    TieDone {
        actor_id: EntityId,
        target_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Friar Tuck finished healing a wounded PC.
    HealDone {
        healer_id: EntityId,
        target_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Robin Hood finished whistling.
    WhistleDone {
        actor_id: EntityId,
        position: ElemPoint2D,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC's Listen entry transition animation completed — the PC
    /// is now in `ActionState::Listening` / `ListenPhase::CountingDown`.
    /// The engine handler sends `PcMessage::SelectAction(Listen)` so
    /// the portrait/action-bar reflects the active ability.
    ListenEntered { actor_id: EntityId },
    /// A PC's Listen exit transition animation completed — clean up:
    /// terminate the driving sequence element and send
    /// `PcMessage::UnselectAction`.
    ListenDone {
        actor_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Stuteley finished the net throw animation.
    ThrowNetDone {
        actor_id: EntityId,
        /// 2D target position for the net projectile.
        target_pos: ElemPoint2D,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Stuteley finished the wasp nest throw animation.
    ThrowWaspNestDone {
        actor_id: EntityId,
        target_pos: ElemPoint2D,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the purse-throw animation.  The engine handler
    /// spawns the purse projectile; its impact handler scatters coins.
    ThrowPurseDone {
        actor_id: EntityId,
        /// 2D ground target the purse arcs toward.
        target_pos: ElemPoint2D,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the apple throw animation.  The engine handler
    /// spawns a bursting apple projectile and decrements apple ammo.
    ThrowAppleDone {
        actor_id: EntityId,
        /// Antagonist entity (soldier, civilian, or FX target) — the
        /// apple is aimed at this entity's eyes / center.
        target: Option<EntityId>,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the stone throw animation.  The engine handler
    /// spawns a bursting stone projectile and decrements stone ammo.
    ThrowStoneDone {
        actor_id: EntityId,
        target: Option<EntityId>,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A VIP PC finished the `Paying` animation on a beggar.  The
    /// engine handler subtracts [`BEGGAR_SALARY`] from the ransom and
    /// launches a [`Command::ReceivePurse`] sequence element on the
    /// beggar.
    ///
    /// [`BEGGAR_SALARY`]: crate::engine::BEGGAR_SALARY
    /// [`Command::ReceivePurse`]: crate::element::Command::ReceivePurse
    PayDone {
        pc_id: EntityId,
        beggar_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A beggar civilian finished the middle `WaitingWithPurse`
    /// animation of the `ReceivePurse` chain.  The engine handler runs
    /// [`EngineInner::reveal_scrolls`] at this point so the minimap
    /// delayed-highlight fires on the Waiting→Transition boundary.
    /// The sequence element is *not* terminated yet — the `Transition`
    /// animation still has to play, ending with
    /// [`AbilityTickResult::ReceivePurseDone`].
    ///
    /// [`EngineInner::reveal_scrolls`]: crate::engine::EngineInner::reveal_scrolls
    ReceivePurseRevealing { beggar_id: EntityId },
    /// A beggar civilian finished the final `Transition` animation of
    /// the `ReceivePurse` chain.  The engine handler terminates the
    /// driving sequence element.
    ReceivePurseDone {
        beggar_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the `Hitting` animation.  The engine handler
    /// launches a [`Command::ReceiveHitDamage`] damage element on the
    /// target with concussion 80 / 150 depending on whether the
    /// attacker's profile carries the HIT_HARD action slot.
    ///
    /// [`Command::ReceiveHitDamage`]: crate::element::Command::ReceiveHitDamage
    HitDone {
        actor_id: EntityId,
        target_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the `Strangling` animation.  The engine handler
    /// launches a full-life-points [`Command::ReceiveDamage`] element
    /// to kill the victim (or, for non-stranglable soldiers, dispatches
    /// an `EventGotHit` stimulus so the soldier retaliates instead).
    ///
    /// [`Command::ReceiveDamage`]: crate::element::Command::ReceiveDamage
    StrangleDone {
        actor_id: EntityId,
        target_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the `Eating` animation.  The engine handler
    /// decrements the Eat / Guzzle ammo counter and adds 40 (Eat) or
    /// 80 (Guzzle) life points, capped at `LIFEPOINTS_PC`.
    EatDone {
        actor_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the `ClimbingUpOnShoulders` animation.  By the time
    /// the engine handler runs the postures are already paired
    /// (`OnShoulders` on the climber, `CarryingOnShoulders` on the
    /// helper) — `begin_climb_on_shoulders` did the latch on init so
    /// that pairing exists from frame 1.  The handler terminates the
    /// driving sequence element and parks the helper on a low-priority
    /// Wait so its AI re-enters the idle loop while frozen-on-shoulders.
    ClimbOnShouldersDone {
        /// The PC that climbed up (executor of the order).
        climber_id: EntityId,
        /// The HelpingToClimb partner now carrying the climber.
        helper_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the `ClimbingDownFromShoulders` animation.  The
    /// engine handler resets postures (`Upright` on the climber,
    /// `HelpingToClimb` on the helper), severs the `pc.carried` /
    /// `human.carrier` cross-references, parks the helper on a Wait,
    /// and relocates the climber to an authorized landing position next
    /// to the helper.
    ClimbDownFromShouldersDone {
        /// The PC that climbed down (executor of the order).
        climber_id: EntityId,
        /// The HelpingToClimb partner that was carrying the climber.
        helper_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    },
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Carry body (Little John)
// ═══════════════════════════════════════════════════════════════════

/// Start picking up an unconscious/dead body.
///
/// Called when `Command::TakeCorpse` is dispatched.
///
/// ## Known gaps
///
/// - **Per-frame anim sync**: the carried entity's pickup animation
///   (`BeingLiftedLittleJohn` / `BeingLiftedPeasantC`) is not
///   synchronized to the carrier's sprite every frame during the
///   transition.  We only sync position/direction in
///   [`sync_carried_positions`].
/// - **Building hulk**: the re-select + hulk start step when picking
///   up in a building sector is applied in the `Command::TakeCorpse`
///   handler in `engine/tick.rs` after `begin_carry` succeeds.
pub fn begin_carry(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    carrier_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    if carrier_id == target_id {
        return BeginResult::Impossible;
    }

    // Validate target: must exist, be human, out-of-order, in a carryable posture.
    let target_valid = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) => {
            if !e.is_human() {
                false
            } else {
                let posture = e.element_data().posture;
                let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                let dead = e.is_dead();
                (unconscious || dead)
                    && matches!(
                        posture,
                        Posture::Lying | Posture::Dead | Posture::DeadBack | Posture::Tied
                    )
            }
        }
        None => false,
    };
    if !target_valid {
        return BeginResult::Impossible;
    }

    // Save target posture (Dead is mapped to DeadBack so the
    // dropped-body posture restored later carries the back-down variant).
    let target_posture = {
        let target = entities[target_id.0 as usize].as_ref().unwrap();
        let p = target.element_data().posture;
        if p == Posture::Dead {
            Posture::DeadBack
        } else {
            p
        }
    };
    let target_pos = {
        let target = entities[target_id.0 as usize].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate carrier: must be a living PC, not already carrying.
    let carrier = match entities
        .get_mut(carrier_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !carrier.is_pc() || carrier.is_dead() {
        return BeginResult::Impossible;
    }
    if carrier.pc_data().is_some_and(|pc| pc.carried.is_some()) {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);

    // Latch `pc.carried` and `pc.carried_posture` on the carrier *now*,
    // before the pickup animation finishes, so the carrier "owns" the
    // body through the entire pickup transition.  A mid-grab cancel
    // (sequence interrupted before the pickup animation finishes) is
    // recognised by `send_condolation_card_pc` — its TAKE_CORPSE arm
    // checks `pc.carried.is_some() && carried_posture != Carried` and
    // force-drops the partially-grabbed body.  Without these set during
    // the transition, the condolation handler would have nothing to drop.
    if let Some(pc) = carrier.pc_data_mut() {
        pc.carried = Some(target_id);
        pc.carried_posture = target_posture;
    }

    // Set up the ability tracker and push the pickup animation order.
    let actor = match carrier.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Carry),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(
        OrderType::TransitionWaitingUprightCarryingCorpse,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target — match the `_iso` aspect convention used by
    // every other ability init face-target so the sprite ends up
    // aligned with what the downstream animation expects.  (The
    // preceding `SEEK` element will already have oriented the PC, but
    // we re-snap here to be defensive.)
    let carrier_pos = carrier.element_data().position_map();
    let dx = target_pos.x - carrier_pos.x;
    let dy = target_pos.y - carrier_pos.y;
    carrier.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    // Carry is now committed. Set up the target:
    // - Back-reference (`human.carrier = carrier_id`) so
    //   `compute_stars_point` and other systems can find the carrier.
    // - The freeze-execution step is performed by the caller via
    //   `EngineInner::actor_freeze_execution(target_id)` after this
    //   returns `Started`, so the cascade-interrupt on the target's
    //   current sequence element happens with the full engine context
    //   available.
    if let Some(Some(target)) = entities.get_mut(target_id.0 as usize) {
        if let Some(human) = target.human_data_mut() {
            human.carrier = Some(carrier_id);
        }
        // Re-enable anti-collision so the carrier-body collision tests
        // fire while the corpse is held.
        if let Some(actor) = target.actor_data_mut() {
            actor.is_ignored_for_anti_collision = false;
        }
    }

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Drop corpse
// ═══════════════════════════════════════════════════════════════════

/// Start dropping a carried body.
///
/// Called when `Command::DropCorpse` is dispatched.
///
/// ## Known gaps
///
/// - **Authorized landing position**: a valid walkable position near
///   the carrier for the dropped body's bounding box is not searched
///   — we place at the carrier's exact position.
/// - **Instant vs animated**: the original drops instantly in building
///   sectors but uses delayed positioning outdoors.  We always use the
///   animation path.
/// - **Per-frame anim sync**: `BeingDroppedLittleJohn` /
///   `BeingDroppedPeasantC` is not synchronized on the carried entity
///   during the drop animation.
pub fn begin_drop(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    carrier_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let carrier = match entities
        .get_mut(carrier_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !carrier.is_pc() {
        return BeginResult::Impossible;
    }

    let carried_id = match carrier.pc_data().and_then(|pc| pc.carried) {
        Some(id) => id,
        None => return BeginResult::Impossible,
    };

    let order_id = alloc_order_id(order_id_counter);
    let actor = match carrier.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Drop),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(carried_id),
        order_id: Some(order_id),
    };
    actor.clear_path();

    let mut order = Order::new(
        OrderType::TransitionCarryingCorpseWaitingUpright,
        0.0,
        0.0,
        order_id,
    );
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Climb on shoulders (Little John mounting helper)
// ═══════════════════════════════════════════════════════════════════

/// Outcome of attempting to begin a climb-on-shoulders.
///
/// Distinct from [`BeginResult`] because there is a third arm: when the
/// helper has no headroom (`can_carry_on_shoulders == false`), the climb
/// element is marked `Impossible` *and* a `LeaveHelpingClimb` element is
/// launched on the helper so they stand back up rather than being stuck
/// in the helping pose forever.
pub enum ClimbResult {
    /// Climb animation started.
    Started,
    /// Climb couldn't begin and no compensating action is needed.
    Impossible,
    /// Helper has no headroom — caller must mark the element Impossible
    /// AND launch `Command::LeaveHelpingClimb` on `helper_id`.
    NoHeadroom { helper_id: EntityId },
}

/// Start a Little John-style climb onto a HelpingToClimb partner's
/// shoulders.
///
/// Called when `Command::ClimbUpOnShoulders` is dispatched.  Headroom
/// check, posture latching, pair-state setup, and order push all happen
/// here at init — there is no separate Instruct phase.
///
/// The order is pushed on the *climber*'s sequence element.  The helper
/// is paired into the climber via `HumanData.carrier`, and the carry
/// back-reference goes the same direction as a corpse-carry: the helper
/// records `PcData.carried = climber_id`, with
/// `carried_posture = OnShoulders` so [`sync_carried_positions`] can
/// distinguish a shoulder-mount from a corpse-carry and drive the
/// helper's animation in lockstep with the climber.
///
/// ## Known gaps
///
/// - **Per-frame turn**: the climber's direction is set once at init.
///   The helper's direction doesn't change while frozen, so a per-frame
///   recompute would be a no-op anyway.
/// - **Display-order pin**: pinning the climber's draw order in front
///   of the helper is handled by `sync_carried_positions` setting
///   `display_order_ref = helper`.
#[allow(clippy::too_many_arguments)]
pub fn begin_climb_on_shoulders(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    climber_id: EntityId,
    helper_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> ClimbResult {
    if climber_id == helper_id {
        return ClimbResult::Impossible;
    }

    // Validate helper: must be a living PC currently in the
    // `HelpingToClimb` posture.  Snapshot the helper's position + 3D
    // position here to feed the headroom check before borrowing the
    // climber.
    let (helper_pos_map, helper_pos_3d, helper_valid) =
        match entities.get(helper_id.0 as usize).and_then(|s| s.as_ref()) {
            Some(e) => {
                let valid = e.is_pc()
                    && !e.is_dead()
                    && e.element_data().posture == Posture::HelpingToClimb;
                (
                    e.element_data().position_map(),
                    e.position_iface().get_position(),
                    valid,
                )
            }
            None => (ElemPoint2D { x: 0.0, y: 0.0 }, Default::default(), false),
        };
    if !helper_valid {
        return ClimbResult::Impossible;
    }

    // Headroom check.  When blocked by a ceiling, the climber's
    // element is Impossible AND the helper gets a `LeaveHelpingClimb`
    // element so they don't stay stuck in the helping pose.
    if !can_carry_on_shoulders(helper_pos_3d, obstacles) {
        return ClimbResult::NoHeadroom { helper_id };
    }

    // Validate climber: must be a living PC, not already busy with an
    // ability, not already on shoulders.
    let climber = match entities
        .get_mut(climber_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return ClimbResult::Impossible,
    };
    if !climber.is_pc() || climber.is_dead() {
        return ClimbResult::Impossible;
    }
    if climber
        .actor_data()
        .is_some_and(|a| a.active_ability.is_active())
    {
        return ClimbResult::Impossible;
    }
    // Latch both posture and the carrier back-reference NOW (not on
    // Done) so `sync_carried_positions` starts driving position/anim
    // from frame 1.
    climber.set_posture(Posture::OnShoulders);
    if let Some(human) = climber.human_data_mut() {
        human.carrier = Some(helper_id);
    }
    // Climber snaps onto the helper before the climb anim plays.
    // `sync_carried_positions` will re-stamp every subsequent frame.
    climber.element_data_mut().set_position_map(helper_pos_map);

    let order_id = alloc_order_id(order_id_counter);
    let actor = climber.actor_data_mut().unwrap();
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ClimbOnShoulders),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(helper_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    // Climber direction is the reverse of the helper's (sector + 8).
    // Helper hasn't been touched yet, so read its direction here and
    // mirror it onto the climber for the lifetime of the climb.
    let helper_dir = entities
        .get(helper_id.0 as usize)
        .and_then(|s| s.as_ref())
        .map(|e| e.element_data().direction())
        .unwrap_or(0);
    if let Some(Some(climber)) = entities.get_mut(climber_id.0 as usize) {
        climber
            .element_data_mut()
            .set_direction_instantly((helper_dir + 8) & 15);
    }

    // Push the climbing animation order on the climber's sequence
    // element.  Direction is locked (already faced manually above) and
    // the helper is the antagonist.
    let mut order = Order::new(
        OrderType::ClimbingUpOnShoulders,
        helper_pos_map.x,
        helper_pos_map.y,
        order_id,
    );
    order.target_actor = Some(helper_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Pair the helper: posture → CarryingOnShoulders, latch
    // `pc.carried = climber` so `sync_carried_positions` can drive the
    // helper's TransitionHelpingClimbingUp sync each frame.  Also face
    // the climber.
    let helper = match entities
        .get_mut(helper_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return ClimbResult::Impossible,
    };
    helper.set_posture(Posture::CarryingOnShoulders);
    if let Some(actor) = helper.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }
    if let Some(pc) = helper.pc_data_mut() {
        pc.carried = Some(climber_id);
        pc.carried_posture = Posture::OnShoulders;
    }
    // Snapshot climber pos for the facing computation.
    let climber_pos = entities
        .get(climber_id.0 as usize)
        .and_then(|s| s.as_ref())
        .map(|e| e.element_data().position_map())
        .unwrap_or(helper_pos_map);
    if let Some(Some(helper)) = entities.get_mut(helper_id.0 as usize) {
        let dx = climber_pos.x - helper_pos_map.x;
        let dy = climber_pos.y - helper_pos_map.y;
        helper.element_data_mut().set_direction_instantly(
            crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
        );
    }

    ClimbResult::Started
}

/// Start the dismount animation for a PC currently `OnShoulders`.
///
/// Called when `Command::ClimbDownFromShoulders` is dispatched.
///
/// The order is pushed on the *climber*'s sequence element.  The helper
/// (carrier) is identified via the climber's `human.carrier`
/// back-reference latched in [`begin_climb_on_shoulders`].  Posture
/// reset, carrier-link severance and landing-position resolution happen
/// in the [`AbilityTickResult::ClimbDownFromShouldersDone`] consumer
/// after the animation reaches its terminated state.
pub fn begin_climb_down_from_shoulders(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    climber_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    // Validate climber: must be a living PC currently OnShoulders with
    // a carrier reference.
    let carrier_id = match entities.get(climber_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) => {
            if !e.is_pc() || e.is_dead() {
                return BeginResult::Impossible;
            }
            if e.element_data().posture != Posture::OnShoulders {
                return BeginResult::Impossible;
            }
            if e.actor_data().is_some_and(|a| a.active_ability.is_active()) {
                return BeginResult::Impossible;
            }
            match e.human_data().and_then(|h| h.carrier) {
                Some(id) => id,
                None => return BeginResult::Impossible,
            }
        }
        None => return BeginResult::Impossible,
    };

    let order_id = alloc_order_id(order_id_counter);
    let climber = match entities
        .get_mut(climber_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    let actor = climber.actor_data_mut().unwrap();
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ClimbDownFromShoulders),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(carrier_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    // Push the climbing-down animation order on the climber's sequence
    // element.  Direction is locked.
    let mut order = Order::new(OrderType::ClimbingDownFromShoulders, 0.0, 0.0, order_id);
    order.target_actor = Some(carrier_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Tie up (all PCs)
// ═══════════════════════════════════════════════════════════════════

/// Start tying up an unconscious enemy.
///
/// Called when `Command::TieCmd` is dispatched.
///
/// ## Known gaps
///
/// - **Tied-up remark**: soldiers don't say a "tied up" remark when
///   tied — no speech system yet.
/// - **Reset target AI**: the target's AI isn't reset to a Wait state
///   after tying.
pub fn begin_tie(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    if actor_id == target_id {
        return BeginResult::Impossible;
    }

    // Validate target: must be unconscious and lying (not already tied).
    let target_valid = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) => {
            let posture = e.element_data().posture;
            let unconscious = e.human_data().is_some_and(|h| h.unconscious);
            unconscious && posture == Posture::Lying
        }
        None => false,
    };
    if !target_valid {
        return BeginResult::Impossible;
    }

    let target_pos = {
        let target = entities[target_id.0 as usize].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate actor: must be alive, human, not already busy.
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_human() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Tie),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Tying, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Heal (Friar Tuck)
// ═══════════════════════════════════════════════════════════════════

/// Start healing a wounded PC.
///
/// Called when `Command::HealCmd` is dispatched.  Self-heal runs
/// `OrderType::Eating` instead of `Healing`; the post-heal speech cue
/// fires from the `HealDone` branch in `engine::combat`.
pub fn begin_heal(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    healer_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    // Validate target: living PC with HP < max, OR an FX target (which
    // just runs the animation so the target's `ActivatedByHeal` script
    // can fire on Done — see `HealDone` in `engine/combat.rs`).
    let target_valid = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) => {
            if e.kind().is_fx_target() {
                true
            } else if e.is_pc() {
                let hp = e.pc_data().map(|pc| pc.life_points).unwrap_or(0);
                hp > 0 && hp < LIFEPOINTS_PC
            } else {
                false
            }
        }
        None => false,
    };
    if !target_valid {
        return BeginResult::Impossible;
    }

    let target_pos = {
        let target = entities[target_id.0 as usize].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate healer: must be alive PC.
    let healer = match entities
        .get_mut(healer_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !healer.is_pc() || healer.is_dead() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match healer.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Heal),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    // Queue the canonical Healing order; `tick_abilities` swaps to
    // `OrderType::Eating` when the target is the healer itself.
    let mut order = Order::new(OrderType::Healing, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face target (unless healing self).
    if healer_id != target_id {
        let healer_pos = healer.element_data().position_map();
        let dx = target_pos.x - healer_pos.x;
        let dy = target_pos.y - healer_pos.y;
        healer.element_data_mut().set_direction_instantly(
            crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
        );
    }

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Whistle (Robin Hood)
// ═══════════════════════════════════════════════════════════════════

/// Start whistling to attract guards.
///
/// Called when `Command::WhistleCmd` is dispatched.
///
/// Arms `whistle_wait_time = TIME_LISTEN_WAIT` (25) so `tick_abilities`
/// can decrement it each frame and `render_listen_ping` can draw the
/// expanding noise ellipse during the final `TIME_LISTEN` (5) frames
/// (the Whistling arm of the shared Listen/Whistle ellipse render).
pub fn begin_whistle(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Whistle),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;
    // Set the wait-time at ability launch: `tick_abilities` decrements
    // it on every frame including the first.
    actor.whistle_wait_time = TIME_LISTEN_WAIT;

    let mut order = Order::new(OrderType::Whistling, 0.0, 0.0, order_id);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Eat (ration)
// ═══════════════════════════════════════════════════════════════════

/// Start eating a ration to recover life points.
///
/// Called when `Command::EatCmd` is dispatched.  The dispatcher (tick.rs)
/// checks Eat ammo > 0 before calling this; on success we queue the
/// `Eating` animation order, and the post-animation effect is applied
/// by the [`AbilityTickResult::EatDone`] handler in `engine::combat`.
pub fn begin_eat(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Eat),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Eating, 0.0, 0.0, order_id);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Hit (punch)
// ═══════════════════════════════════════════════════════════════════

/// Start a hit (punch) attack against a human target.
///
/// Called when `Command::HitCmd` is dispatched.
///
/// The attacker plays the `Hitting` animation; on completion
/// [`tick_abilities`] emits [`AbilityTickResult::HitDone`], and the
/// engine handler (`engine::combat`) launches a
/// [`Command::ReceiveHitDamage`] damage element on the target with
/// concussion 80 (`Action::Hit`) or 150 (`Action::HitHard`) based on
/// whether the attacker's profile carries the HitHard action slot.
///
/// [`Command::ReceiveHitDamage`]: crate::element::Command::ReceiveHitDamage
pub fn begin_hit(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    if actor_id == target_id {
        return BeginResult::Impossible;
    }

    // The completion path asserts the antagonist is human; gate on the
    // same condition up-front instead of panicking later.
    let (target_pos, target_alive) =
        match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
            Some(e) if e.is_human() => (e.element_data().position_map(), !e.is_dead()),
            _ => return BeginResult::Impossible,
        };
    if !target_alive {
        return BeginResult::Impossible;
    }

    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_human() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Hit),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Hitting, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity
        .element_data_mut()
        .set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15(dx, dy));

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Strangle (any PC)
// ═══════════════════════════════════════════════════════════════════

/// Start strangling an NPC.
///
/// Called when `Command::StrangleCmd` is dispatched.
///
/// The attacker plays the `Strangling` animation; on completion
/// [`tick_abilities`] emits [`AbilityTickResult::StrangleDone`], and
/// the engine handler launches a full-life-points
/// [`Command::ReceiveDamage`] element that kills the victim.
///
/// [`Command::ReceiveDamage`]: crate::element::Command::ReceiveDamage
pub fn begin_strangle(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    if actor_id == target_id {
        return BeginResult::Impossible;
    }

    // Invalid-target rejection happens in
    // `EngineInner::check_sequence_element_validity`, which runs before
    // dispatch.  This helper just rejects any non-living-human-NPC.
    let target_pos = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) if e.is_human() && !e.is_dead() && !e.is_pc() => e.element_data().position_map(),
        _ => return BeginResult::Impossible,
    };

    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !actor_entity.is_pc() || actor_entity.is_dead() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Strangle),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Strangling, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the victim — set the direction for both the strangler and
    // the victim; the victim's direction is matched when the kill fires.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    let facing = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
    actor_entity
        .element_data_mut()
        .set_direction_instantly(facing);

    // Face the victim the same way the strangler faces, and lock the
    // victim's AI with `AiLockFlags::FREEZE` for the duration of the
    // strangle.  The matching unlock fires from
    // `send_condolation_card_pc` when the strangle element terminates.
    //
    // Additionally: when the antagonist is an NPC currently moving,
    // dispatch an `EventStop` stimulus so its AI registers the imminent
    // strangle and halts.  Queue it via the cross-NPC drain so it fires
    // later this tick (`process_pending_cross_npc_actions` runs after
    // dispatch).  Push the stimulus BEFORE setting the FREEZE lock —
    // ordering matters: the stimulus is a sequence transition (earlier)
    // and the lock is part of the strangle init (later).
    if let Some(Some(victim)) = entities.get_mut(target_id.0 as usize) {
        victim.element_data_mut().set_direction_instantly(facing);
        let is_moving = victim
            .actor_data()
            .map(|a| a.action_state.is_moving())
            .unwrap_or(false);
        if let Some(ai) = victim.ai_controller_mut() {
            if is_moving {
                ai.pending_cross_npc_actions
                    .push(crate::ai::CrossNpcAction::SendStimulus {
                        target: target_id.0,
                        stimulus_type: crate::ai::StimulusType::EventStop,
                        info: crate::ai::StimulusInfo::None,
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                    });
            }
            ai.non_script_lock(crate::ai::AiLockFlags::FREEZE);
        }
    }

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Listen (any PC)
// ═══════════════════════════════════════════════════════════════════

/// Start the Listen ability.
///
/// Called when `Command::EnterListen` is dispatched.  The ability
/// drives a three-phase flow
/// (`TransitionWaitingUprightListening` → `Listening` →
/// `TransitionListeningWaitingUpright`) via [`ListenPhase`] + the
/// `active_ability` / `tick_abilities` machinery:
///
/// 1. `begin_listen` sets `ListenPhase::EnterTransition` and starts
///    the entry transition animation.
/// 2. `tick_abilities` drives the transition sprite; on `Done` it
///    flips `action_state = Listening`, `ListenPhase::CountingDown`,
///    and returns an `AbilityTickResult::ListenEntered` so the
///    engine can send a `SelectAction(Listen)` PC message.
/// 3. `engine/ai.rs` section 2a arms `listen_wait_time`, decrements
///    it each frame, fires the one-shot reveal + FX-target `Heard()`
///    when it reaches 0, and advances to `ListenPhase::ExitTransition`.
/// 4. `tick_abilities` drives the exit transition sprite; on `Done`
///    it cleans up and returns `AbilityTickResult::ListenDone` so
///    the engine sends `UnselectAction` and terminates the driving
///    sequence element.
pub fn begin_listen(
    entities: &mut [Option<Entity>],
    profiles: &crate::profiles::ProfileManager,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }

    // Honour the Listen portrait slot. C++ disabled action masks are
    // indexed by profile action slot, not by the action enum value.
    if let Some(pc) = actor_entity.pc_data() {
        let Some(profile) = profiles.get_character(pc.profile_index) else {
            return BeginResult::Impossible;
        };
        if let Some(idx) =
            crate::inventory::find_action_slot(profile, crate::profiles::Action::Listen)
            && (pc.disabled_actions.get(idx).copied().unwrap_or(false)
                || pc.disabled_actions_temp.get(idx).copied().unwrap_or(false))
        {
            return BeginResult::Impossible;
        }
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Listen),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    // Stay in Waiting during the entry transition so the idle-pose
    // driver doesn't fight the transition animation with a looping
    // LISTENING pose.  `tick_abilities` will flip to
    // `ActionState::Listening` on transition `Done`.
    actor.action_state = ActionState::Waiting;
    actor.listen_phase = ListenPhase::EnterTransition;
    actor.listen_wait_time = 0;

    let mut order = Order::new(
        OrderType::TransitionWaitingUprightListening,
        0.0,
        0.0,
        order_id,
    );
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Leave Listen (cancel mid-countdown)
// ═══════════════════════════════════════════════════════════════════

/// Cancel an in-progress Listen ability by flipping the phase to
/// `ExitTransition`.
///
/// Called when `Command::LeaveListen` is dispatched — e.g. the
/// player toggles Listen off mid-countdown.
///
/// Unlike other `begin_*` functions, `LeaveListen` does not create a
/// new `active_ability` — the still-active EnterListen ability tracks
/// the exit animation.  This function just flips the phase and bumps
/// `order_id` so `perform_action` starts the exit animation fresh.
/// The `LeaveListen` sequence element itself is terminated
/// immediately by the engine dispatcher (it has no ongoing task).
///
/// Returns `true` if the PC was in a cancellable phase and the
/// transition was armed; `false` otherwise (already exiting or not
/// listening — safe no-op).
pub fn begin_leave_listen(
    entities: &mut [Option<Entity>],
    actor_id: EntityId,
    order_id_counter: &mut u32,
) -> bool {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return false,
    };
    if !actor_entity.is_pc() {
        return false;
    }
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return false,
    };
    if !matches!(actor.active_ability.kind, Some(AbilityKind::Listen)) {
        return false;
    }
    match actor.listen_phase {
        ListenPhase::CountingDown | ListenPhase::EnterTransition => {
            actor.listen_phase = ListenPhase::ExitTransition;
            actor.listen_wait_time = 0;
            actor.active_ability.order_id = Some(alloc_order_id(order_id_counter));
            // Action state stays Listening until the exit transition
            // flips it to Waiting on Done (handled in tick_abilities).
            true
        }
        ListenPhase::ExitTransition | ListenPhase::Inactive => false,
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Throw net (Stuteley)
// ═══════════════════════════════════════════════════════════════════

/// Start the net-throwing animation.
///
/// Called when `Command::ThrowNet` is dispatched.
///
/// ## Known gaps
///
/// - **Gradual turning**: the original freezes the throw on its first
///   frame until the actor finishes rotating to face the target.  We
///   set direction instantly.
pub fn begin_throw_net(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: ElemPoint2D,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ThrowNet),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None, // ground target, not entity
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::ThrowingNet, target_pos.x, target_pos.y, order_id);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target position.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Throw apple / stone (any PC)
// ═══════════════════════════════════════════════════════════════════

/// Start an apple-throw animation.
///
/// Called when `Command::ThrowApple` is dispatched.  The apple itself
/// is spawned when the animation completes — see
/// [`AbilityTickResult::ThrowAppleDone`] and the engine-side handler.
pub fn begin_throw_apple(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    begin_throw_at_entity(
        entities,
        sequence_manager,
        actor_id,
        target,
        seq_id,
        elem_idx,
        order_id_counter,
        AbilityKind::ThrowApple,
        OrderType::ThrowingApple,
    )
}

/// Start a stone-throw animation.
///
/// Called when `Command::ThrowStone` is dispatched.
pub fn begin_throw_stone(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    begin_throw_at_entity(
        entities,
        sequence_manager,
        actor_id,
        target,
        seq_id,
        elem_idx,
        order_id_counter,
        AbilityKind::ThrowStone,
        OrderType::ThrowingStone,
    )
}

/// Shared begin path for entity-targeted throws (apple, stone).  The
/// antagonist entity is stored on `ActiveAbility.target` so the
/// completion handler can compute the target's eyes / center as the
/// trajectory endpoint.
#[allow(clippy::too_many_arguments)]
fn begin_throw_at_entity(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
    kind: AbilityKind,
    order_type: OrderType,
) -> BeginResult {
    let target_pos = match entities.get(target_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e) => e.element_data().position_map(),
        None => return BeginResult::Impossible,
    };
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }
    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(kind),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(order_type, target_pos.x, target_pos.y, order_id);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Throw wasp nest (Stuteley)
// ═══════════════════════════════════════════════════════════════════

/// Start the wasp-nest throw animation.
///
/// Called when `Command::ThrowWaspNest` is dispatched.
///
/// ## Known gaps
///
/// Same as [`begin_throw_net`] — gradual turning not ported.
pub fn begin_throw_wasp_nest(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: ElemPoint2D,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }
    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ThrowWaspNest),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(
        OrderType::ThrowingWaspNest,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target position.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Throw purse (any PC)
// ═══════════════════════════════════════════════════════════════════

/// Start the purse-throw animation.
///
/// Called when `Command::ThrowPurse` is dispatched.
///
/// ## Known gaps
///
/// Same as [`begin_throw_net`] — gradual turning not ported.
pub fn begin_throw_purse(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: ElemPoint2D,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities
        .get_mut(actor_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }
    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ThrowPurse),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(
        OrderType::ThrowingPurse,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face the target position.
    let actor_pos = actor_entity.element_data().position_map();
    let dx = target_pos.x - actor_pos.x;
    let dy = target_pos.y - actor_pos.y;
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
    );

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Pay (VIP PC pays a beggar)
// ═══════════════════════════════════════════════════════════════════

/// Start the `Paying` animation for a VIP PC handing money to a beggar.
///
/// Faces away from the beggar, plays the Paying animation, and — on
/// completion — deducts the beggar salary and launches a `ReceivePurse`
/// sequence element on the beggar.  The post-completion side effects
/// live in [`AbilityTickResult::PayDone`] because they need the
/// EngineInner-level borrow.
///
/// The PC's "give money" speech cue is handled by the engine at
/// dispatch time — it's cheaper to fire once at command launch than
/// to thread a side-effect out of `tick_abilities`.
pub fn begin_pay(
    entities: &mut [Option<Entity>],
    sequence_manager: &mut SequenceManager,
    pc_id: EntityId,
    beggar_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    if pc_id == beggar_id {
        return BeginResult::Impossible;
    }

    // Validate beggar: civilian, alive, conscious, non-scroll-attached.
    let beggar_valid = match entities.get(beggar_id.0 as usize).and_then(|s| s.as_ref()) {
        Some(e @ Entity::Civilian(c)) => {
            !e.is_dead()
                && !c.human.unconscious
                && !c.npc.scroll_attached
                && c.civilian.beggar_scroll_sets.is_some()
        }
        _ => false,
    };
    if !beggar_valid {
        return BeginResult::Impossible;
    }

    let beggar_direction = entities[beggar_id.0 as usize]
        .as_ref()
        .unwrap()
        .element_data()
        .direction();

    let order_id = alloc_order_id(order_id_counter);
    let pc_entity = match entities.get_mut(pc_id.0 as usize).and_then(|s| s.as_mut()) {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if pc_entity.is_dead() || !pc_entity.is_pc() {
        return BeginResult::Impossible;
    }
    let pc_pos = pc_entity.element_data().position_map();

    let actor = match pc_entity.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::Pay),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(beggar_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Paying, pc_pos.x, pc_pos.y, order_id);
    order.target_actor = Some(beggar_id.0);
    order.compute_direction = false;
    order.lock_ai = true;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Face opposite to beggar.
    pc_entity
        .element_data_mut()
        .set_direction_instantly((beggar_direction + 8).rem_euclid(16));

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — ReceivePurse (beggar response chain)
// ═══════════════════════════════════════════════════════════════════

/// Start the three-animation `ReceivePurse` chain on a beggar civilian.
///
/// The chain runs three orders back-to-back: `ReceivingPurse` →
/// `WaitingWithPurse` → transition-back-to-upright.  We track the
/// current phase in [`ActorData::receive_purse_phase`] so the
/// `tick_abilities` dispatch can fire [`EngineInner::reveal_scrolls`]
/// on the Waiting→Transition boundary.
///
/// [`EngineInner::reveal_scrolls`]: crate::engine::EngineInner::reveal_scrolls
pub fn begin_receive_purse(
    entities: &mut [Option<Entity>],
    beggar_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let beggar = match entities
        .get_mut(beggar_id.0 as usize)
        .and_then(|s| s.as_mut())
    {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    match beggar {
        Entity::Civilian(c) if c.civilian.beggar_scroll_sets.is_some() => {}
        _ => return BeginResult::Impossible,
    }
    if beggar.is_dead() || beggar.human_data().is_some_and(|h| h.unconscious) {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match beggar.actor_data_mut() {
        Some(a) => a,
        None => return BeginResult::Impossible,
    };
    if actor.active_ability.is_active() {
        return BeginResult::Impossible;
    }
    // The beggar must already be idling in `Waiting` before the
    // purse-chain can begin.  Beggars are stationary NPCs so this is
    // nearly always satisfied; reject the command on anything else
    // (e.g. a still-walking beggar) so the sequence manager can
    // surface it as `Impossible` rather than silently forcing the
    // state.
    if actor.action_state != ActionState::Waiting {
        return BeginResult::Impossible;
    }
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ReceivePurse),
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.receive_purse_phase = ReceivePursePhase::Receiving;

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame ability tick
// ═══════════════════════════════════════════════════════════════════

/// Map [`AbilityKind`] to the [`OrderType`] that drives its animation.
fn ability_order_type(kind: AbilityKind) -> OrderType {
    match kind {
        AbilityKind::Carry => OrderType::TransitionWaitingUprightCarryingCorpse,
        AbilityKind::Drop => OrderType::TransitionCarryingCorpseWaitingUpright,
        AbilityKind::Tie => OrderType::Tying,
        AbilityKind::Heal => OrderType::Healing,
        AbilityKind::Whistle => OrderType::Whistling,
        AbilityKind::ThrowNet => OrderType::ThrowingNet,
        AbilityKind::ThrowWaspNest => OrderType::ThrowingWaspNest,
        AbilityKind::ThrowPurse => OrderType::ThrowingPurse,
        AbilityKind::ThrowApple => OrderType::ThrowingApple,
        AbilityKind::ThrowStone => OrderType::ThrowingStone,
        AbilityKind::Pay => OrderType::Paying,
        AbilityKind::Hit => OrderType::Hitting,
        AbilityKind::Strangle => OrderType::Strangling,
        AbilityKind::Eat => OrderType::Eating,
        AbilityKind::ClimbOnShoulders => OrderType::ClimbingUpOnShoulders,
        AbilityKind::ClimbDownFromShoulders => OrderType::ClimbingDownFromShoulders,
        AbilityKind::Listen | AbilityKind::ReceivePurse => unreachable!(
            "{kind:?} is handled inline in tick_abilities — \
             ability_order_type should never be called for it"
        ),
    }
}

/// Advance ability animations for every actor with an [`ActiveAbility`].
///
/// Returns a list of results for abilities that completed this frame.
/// The engine applies cross-entity effects from these results after the
/// mutable borrow on `entities` is released.
pub fn tick_abilities(
    entities: &mut [Option<Entity>],
    sequence_manager: &SequenceManager,
    order_id_counter: &mut u32,
) -> Vec<AbilityTickResult> {
    let mut results = Vec::new();

    for (idx, slot) in entities.iter_mut().enumerate() {
        let entity = match slot {
            Some(e) => e,
            None => continue,
        };
        let actor = match entity.actor_data() {
            Some(a) => a,
            None => continue,
        };
        if !actor.active_ability.is_active() {
            continue;
        }

        let ability = actor.active_ability.clone();
        let kind = ability.kind.unwrap(); // safe: is_active() checked

        // ── Listen: phase-aware animation dispatch ──
        //
        // Listen has three animation phases tracked by
        // `ActorData::listen_phase`.  The entry and exit transitions
        // are one-shot animations driven here; the middle CountingDown
        // phase is a loop driven by the idle-pose animation driver
        // plus the `listen_wait_time` countdown in
        // `engine/ai.rs` section 2a.
        if kind == AbilityKind::Listen {
            let phase = actor.listen_phase;
            let order_type = match phase {
                ListenPhase::EnterTransition => OrderType::TransitionWaitingUprightListening,
                ListenPhase::ExitTransition => OrderType::TransitionListeningWaitingUpright,
                // Countdown phase: the looping LISTENING pose plays
                // via the idle-pose driver; the ai.rs countdown
                // handles timing + the phase flip to ExitTransition.
                ListenPhase::CountingDown => continue,
                ListenPhase::Inactive => {
                    // Shouldn't happen with an active ability; be
                    // defensive and clear the stale ability state.
                    tracing::warn!(
                        entity = idx,
                        "tick_abilities: AbilityKind::Listen with ListenPhase::Inactive — clearing"
                    );
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.active_ability.clear();
                    }
                    continue;
                }
            };
            let direction = u16::try_from(entity.element_data().direction()).unwrap_or(0);
            let order_id = ability.order_id;

            let motion = {
                let elem = entity.element_data_mut();
                elem.sprite.perform_action(
                    order_id,
                    order_type,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                )
            };
            if !matches!(
                motion,
                SpriteMotionState::Done
                    | SpriteMotionState::Terminated
                    | SpriteMotionState::Aborted
            ) {
                continue;
            }

            // Animation completed — advance the phase machine.
            let entity_id = EntityId(idx as u32);
            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => continue,
            };
            match phase {
                ListenPhase::EnterTransition => {
                    // Switch to the listening pose (driven by
                    // animation.rs idle-pose fallback) and hand off
                    // to the ai.rs countdown.
                    actor.action_state = ActionState::Listening;
                    actor.listen_phase = ListenPhase::CountingDown;
                    actor.listen_wait_time = 0; // ai.rs arms to 25 on next tick
                    // Bump the order id so the next perform_action
                    // call (for the exit transition) starts fresh.
                    actor.active_ability.order_id = Some(alloc_order_id(order_id_counter));
                    results.push(AbilityTickResult::ListenEntered {
                        actor_id: entity_id,
                    });
                }
                ListenPhase::ExitTransition => {
                    let seq_id = actor
                        .active_ability
                        .sequence_id
                        .expect("Listen ability must carry a sequence id");
                    let elem_idx = actor.active_ability.element_index;
                    actor.active_ability.clear();
                    actor.action_state = ActionState::Waiting;
                    actor.listen_phase = ListenPhase::Inactive;
                    actor.listen_wait_time = 0;
                    results.push(AbilityTickResult::ListenDone {
                        actor_id: entity_id,
                        seq_id,
                        elem_idx,
                    });
                }
                _ => unreachable!(),
            }
            continue;
        }

        // ── ReceivePurse: phase-aware animation dispatch ──
        //
        // Three sequential one-shot animations play back-to-back:
        // `ReceivingPurse` → `WaitingWithPurse` → transition-back.  On
        // the Waiting→Transition boundary we emit
        // `ReceivePurseRevealing` so the engine can run
        // `reveal_scrolls`; on Transition→Inactive we emit
        // `ReceivePurseDone` to terminate the driving sequence element.
        if kind == AbilityKind::ReceivePurse {
            let phase = actor.receive_purse_phase;
            let order_type = match phase {
                ReceivePursePhase::Receiving => OrderType::ReceivingPurse,
                ReceivePursePhase::Waiting => OrderType::WaitingWithPurse,
                ReceivePursePhase::Transition => {
                    OrderType::TransitionWaitingWithPurseWaitingUpright
                }
                ReceivePursePhase::Inactive => {
                    tracing::warn!(
                        entity = idx,
                        "tick_abilities: AbilityKind::ReceivePurse with \
                         ReceivePursePhase::Inactive — clearing"
                    );
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.active_ability.clear();
                    }
                    continue;
                }
            };
            let direction = u16::try_from(entity.element_data().direction()).unwrap_or(0);
            let order_id = ability.order_id;

            let motion = {
                let elem = entity.element_data_mut();
                elem.sprite.perform_action(
                    order_id,
                    order_type,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                )
            };
            if !matches!(
                motion,
                SpriteMotionState::Done
                    | SpriteMotionState::Terminated
                    | SpriteMotionState::Aborted
            ) {
                continue;
            }

            let entity_id = EntityId(idx as u32);
            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => continue,
            };
            match phase {
                ReceivePursePhase::Receiving => {
                    actor.receive_purse_phase = ReceivePursePhase::Waiting;
                    // Fresh order id for the next animation so the sprite
                    // state machine treats it as a new action.
                    actor.active_ability.order_id = Some(alloc_order_id(order_id_counter));
                }
                ReceivePursePhase::Waiting => {
                    actor.receive_purse_phase = ReceivePursePhase::Transition;
                    actor.active_ability.order_id = Some(alloc_order_id(order_id_counter));
                    // Timing-critical: fire the reveal the moment
                    // WaitingWithPurse ends, before the transition
                    // animation starts.
                    results.push(AbilityTickResult::ReceivePurseRevealing {
                        beggar_id: entity_id,
                    });
                }
                ReceivePursePhase::Transition => {
                    let seq_id = actor
                        .active_ability
                        .sequence_id
                        .expect("ReceivePurse ability must carry a sequence id");
                    let elem_idx = actor.active_ability.element_index;
                    actor.active_ability.clear();
                    actor.action_state = ActionState::Waiting;
                    actor.receive_purse_phase = ReceivePursePhase::Inactive;
                    results.push(AbilityTickResult::ReceivePurseDone {
                        beggar_id: entity_id,
                        seq_id,
                        elem_idx,
                    });
                }
                ReceivePursePhase::Inactive => unreachable!(),
            }
            continue;
        }

        let order_id = ability.order_id;
        // Self-heal swaps Healing → Eating; all other abilities use
        // the canonical per-kind animation.
        let entity_id_here = EntityId(idx as u32);
        let order_type = if kind == AbilityKind::Heal && ability.target == Some(entity_id_here) {
            OrderType::Eating
        } else {
            ability_order_type(kind)
        };
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or(0);

        // Drive the animation through the sprite state machine.
        let motion = {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                order_id,
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };

        // Whistle wait-time countdown.  Drives the expanding
        // noise-ellipse render in `render_listen_ping`; armed to
        // `TIME_LISTEN_WAIT` in `begin_whistle`.
        if kind == AbilityKind::Whistle {
            let actor = entity.actor_data_mut().unwrap();
            if actor.whistle_wait_time != 0 {
                actor.whistle_wait_time -= 1;
            }
        }

        // Only act on completion states.
        if !matches!(
            motion,
            SpriteMotionState::Done | SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            continue;
        }

        // Animation finished — collect the result and clear the ability.
        let entity_id = EntityId(idx as u32);
        let actor_pos = entity.element_data().position_map();
        let actor_direction = u16::try_from(entity.element_data().direction()).unwrap_or(0);

        // Read carried_posture before clearing (needed for Drop).
        let carried_posture = entity
            .pc_data()
            .map(|pc| pc.carried_posture)
            .unwrap_or(Posture::Lying);

        // Clear ability state and reset actor.
        let actor = entity.actor_data_mut().unwrap();
        let seq_id = actor.active_ability.sequence_id.unwrap();
        let elem_idx = actor.active_ability.element_index;
        actor.active_ability.clear();
        actor.action_state = ActionState::Waiting;
        // Whistle countdown should already be 0 by the time the
        // animation completes (TIME_LISTEN_WAIT < whistle anim length),
        // but clamp defensively so a follow-up whistle can re-arm
        // cleanly in `begin_whistle`.
        if kind == AbilityKind::Whistle {
            actor.whistle_wait_time = 0;
        }

        let result = match kind {
            AbilityKind::Carry => {
                // Set carrier posture (target posture set by engine).
                entity.set_posture(Posture::CarryingCorpse);
                AbilityTickResult::CarryDone {
                    carrier_id: entity_id,
                    target_id: ability.target.unwrap(),
                    carried_posture,
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::Drop => {
                entity.set_posture(Posture::Upright);
                AbilityTickResult::DropDone {
                    carrier_id: entity_id,
                    target_id: ability.target.unwrap(),
                    drop_posture: carried_posture,
                    carrier_pos: actor_pos,
                    carrier_direction: actor_direction,
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::Tie => AbilityTickResult::TieDone {
                actor_id: entity_id,
                target_id: ability.target.unwrap(),
                seq_id,
                elem_idx,
            },
            AbilityKind::Heal => AbilityTickResult::HealDone {
                healer_id: entity_id,
                target_id: ability.target.unwrap(),
                seq_id,
                elem_idx,
            },
            AbilityKind::Whistle => AbilityTickResult::WhistleDone {
                actor_id: entity_id,
                position: actor_pos,
                seq_id,
                elem_idx,
            },
            AbilityKind::Pay => AbilityTickResult::PayDone {
                pc_id: entity_id,
                beggar_id: ability
                    .target
                    .expect("AbilityKind::Pay must carry a beggar target (set in begin_pay)"),
                seq_id,
                elem_idx,
            },
            AbilityKind::Listen | AbilityKind::ReceivePurse => unreachable!(
                "{kind:?} is handled by the phase-aware inline branch earlier \
                 in tick_abilities and never reaches the generic completion match"
            ),
            AbilityKind::ThrowNet => {
                // Target position was stored in the order on the
                // owning sequence element.
                let target_pos = sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| ElemPoint2D {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or(actor_pos);
                AbilityTickResult::ThrowNetDone {
                    actor_id: entity_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::ThrowWaspNest => {
                let target_pos = sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| ElemPoint2D {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or(actor_pos);
                AbilityTickResult::ThrowWaspNestDone {
                    actor_id: entity_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::ThrowPurse => {
                let target_pos = sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                    .map(|o| ElemPoint2D {
                        x: o.target_x,
                        y: o.target_y,
                    })
                    .unwrap_or(actor_pos);
                AbilityTickResult::ThrowPurseDone {
                    actor_id: entity_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::ThrowApple => AbilityTickResult::ThrowAppleDone {
                actor_id: entity_id,
                target: ability.target,
                seq_id,
                elem_idx,
            },
            AbilityKind::ThrowStone => AbilityTickResult::ThrowStoneDone {
                actor_id: entity_id,
                target: ability.target,
                seq_id,
                elem_idx,
            },
            AbilityKind::Hit => AbilityTickResult::HitDone {
                actor_id: entity_id,
                target_id: ability
                    .target
                    .expect("AbilityKind::Hit must carry a target (set in begin_hit)"),
                seq_id,
                elem_idx,
            },
            AbilityKind::Strangle => AbilityTickResult::StrangleDone {
                actor_id: entity_id,
                target_id: ability
                    .target
                    .expect("AbilityKind::Strangle must carry a target (set in begin_strangle)"),
                seq_id,
                elem_idx,
            },
            AbilityKind::Eat => AbilityTickResult::EatDone {
                actor_id: entity_id,
                seq_id,
                elem_idx,
            },
            AbilityKind::ClimbOnShoulders => {
                // Postures were latched on init; nothing to flip here.
                // Helper is parked on a Wait by the engine handler so
                // its frozen-execution doesn't block subsequent
                // climb-down arbitration.
                AbilityTickResult::ClimbOnShouldersDone {
                    climber_id: entity_id,
                    helper_id: ability.target.expect(
                        "AbilityKind::ClimbOnShoulders must carry a helper target \
                         (set in begin_climb_on_shoulders)",
                    ),
                    seq_id,
                    elem_idx,
                }
            }
            AbilityKind::ClimbDownFromShoulders => {
                // Posture reset / carrier-link severance / landing-pos
                // resolution happen in the engine consumer (only on
                // animation-terminated, not the per-frame Done states).
                AbilityTickResult::ClimbDownFromShouldersDone {
                    climber_id: entity_id,
                    helper_id: ability.target.expect(
                        "AbilityKind::ClimbDownFromShoulders must carry a helper target \
                         (set in begin_climb_down_from_shoulders)",
                    ),
                    seq_id,
                    elem_idx,
                }
            }
        };

        results.push(result);
    }

    results
}

// ═══════════════════════════════════════════════════════════════════
//  Carried entity position sync
// ═══════════════════════════════════════════════════════════════════

/// Snapshot of carrier state needed to drive the carried entity's
/// sprite each frame.  Collected in one pass, then applied in a second
/// pass to avoid overlapping mutable borrows on the entity slice.
struct CarrierSnapshot {
    carrier_id: EntityId,
    target_id: EntityId,
    pos: ElemPoint2D,
    carrier_dir: i16,
    layer: u16,
    /// Carrier's current sight obstacle (copied onto the carried
    /// entity so its anti-collision and reachability tests use the
    /// same obstacle as the carrier).
    obstacle_index: Option<ObstacleHandle>,
    /// Carrier's current sector (copied onto the carried entity so
    /// sector-driven systems agree on which sector both occupy).
    sector: Option<crate::position_interface::SectorHandle>,
    /// Carrier's current material (derived from its obstacle).
    material: GameMaterial,
    /// Carrier's plane Z coefficients (from its `PositionInterface`) — used
    /// to reproject the carried sprite onto the correct elevation/tilt plane.
    plane: Option<PlaneZCoeffs>,
    /// The carrier's current sprite animation — determines which carry
    /// phase (lift / waiting / walking / drop) the carried entity plays.
    carrier_last_action: OrderType,
    /// Frame state to synchronize with for lift/waiting/drop phases.
    carrier_frame: u16,
    carrier_frame_count: u16,
    /// True if the carrier has `Action::LittleJohnCarry` or
    /// `Action::FarmerCarry` as a contextual action — selects the
    /// LittleJohn-style carry animations (vs peasant-C style).
    little_john_style: bool,
    /// Posture stored on the carrier's `pc.carried_posture` — determines
    /// whether this is a corpse-carry (target on `Posture::Carried` /
    /// `DeadBack` / etc.) or a shoulder-mount (`Posture::OnShoulders`).
    /// The two modes share position/obstacle/plane copy but differ in
    /// animation sync direction (helper drives climber for carry; climber
    /// drives helper for climb-up).
    carried_posture: Posture,
    /// Climber's current sprite state — used in the OnShoulders branch
    /// to drive the helper's `TransitionHelpingClimbingUp` synchronized
    /// animation.  Read from the carried entity's sprite during the
    /// snapshot pass.
    target_last_action: OrderType,
    target_frame: u16,
    target_frame_count: u16,
}

/// Keep carried entities positioned on top of their carrier and drive
/// their sprite animation synchronized with the carrier.
///
/// Called every frame from the engine tick.  For each PC that has
/// `PcData.carried == Some(target_id)`, copies the carrier's map
/// position/direction to the carried entity and forces the carried's
/// sprite to play the appropriate `BeingLifted*` / `BeingCarried*` /
/// `BeingDropped*` animation depending on which carry phase the carrier
/// is in (lift transition / waiting / walking / drop transition).
pub fn sync_carried_positions(
    entities: &mut [Option<Entity>],
    profiles: &crate::profiles::ProfileManager,
) {
    // Collect carrier snapshots first to avoid borrow conflicts.
    let mut snapshots: Vec<CarrierSnapshot> = Vec::new();
    for (idx, slot) in entities.iter().enumerate() {
        let entity = match slot {
            Some(e) => e,
            None => continue,
        };
        let Some(pc) = entity.pc_data() else { continue };
        let Some(target_id) = pc.carried else {
            continue;
        };
        let carrier_id = EntityId(idx as u32);

        let elem = entity.element_data();

        // Check the carrier's profile for LittleJohnCarry or the
        // equivalent FarmerCarry.
        let little_john_style = profiles
            .get_character(pc.profile_index)
            .map(|cp| {
                cp.contextual_actions.iter().any(|&a| {
                    matches!(
                        a,
                        crate::profiles::Action::LittleJohnCarry
                            | crate::profiles::Action::FarmerCarry
                    )
                })
            })
            .unwrap_or(false);

        let (last_action, frame, frame_count) = {
            let s = &elem.sprite;
            (s.last_action, s.current_frame, s.frame_count)
        };

        // Carrier's plane lives on its `PositionInterface`.  It was resolved
        // from the carrier's current `obstacle_index` when the carrier last
        // crossed onto that obstacle; copying it directly avoids having to
        // re-resolve from the sight-obstacle list here.
        let plane = entity.position_iface().get_plane().copied();

        let carried_posture = pc.carried_posture;

        // Snapshot the carried entity's sprite too so the OnShoulders
        // branch can sync the helper's `TransitionHelpingClimbingUp` to
        // the climber's `ClimbingUpOnShoulders`.  For corpse-carry this is
        // unused; the carrier-driven path overwrites these fields anyway.
        let (target_last_action, target_frame, target_frame_count) = entities
            .get(target_id.0 as usize)
            .and_then(|s| s.as_ref())
            .map(|e| {
                let s = &e.element_data().sprite;
                (s.last_action, s.current_frame, s.frame_count)
            })
            .unwrap_or((OrderType::WaitingUpright, 0, 0));

        snapshots.push(CarrierSnapshot {
            carrier_id,
            target_id,
            pos: elem.position_map(),
            carrier_dir: elem.direction(),
            layer: elem.layer(),
            obstacle_index: elem.obstacle_index(),
            sector: elem.sector(),
            material: elem.material(),
            plane,
            carrier_last_action: last_action,
            carrier_frame: frame,
            carrier_frame_count: frame_count,
            little_john_style,
            carried_posture,
            target_last_action,
            target_frame,
            target_frame_count,
        });
    }

    // Apply to each carried entity.
    for snap in snapshots {
        let on_shoulders = snap.carried_posture == Posture::OnShoulders;

        let Some(Some(target)) = entities.get_mut(snap.target_id.0 as usize) else {
            continue;
        };

        // For corpse-carry, the carried body's facing lags the carrier
        // by 4 sectors `(carrier_dir - 4) & 15`.  For climb-on-shoulders
        // the climber faces the *opposite* direction
        // `(carrier_dir + 8) & 15`.
        let carried_dir_i16 = if on_shoulders {
            (snap.carrier_dir + 8) & 15
        } else {
            snap.carrier_dir.wrapping_sub(4) & 15
        };
        let carried_dir_u16 = carried_dir_i16 as u16;

        // Position + layer + direction + obstacle + sector + material:
        // copy all five fields from the carrier, then reproject onto
        // the new plane below.
        {
            let elem = target.element_data_mut();
            elem.set_position_map(snap.pos);
            elem.set_layer(snap.layer);
            elem.set_direction_instantly(carried_dir_i16);
            // Obstacle/plane assignment is handled below via
            // `pi.set_obstacle(snap.obstacle_index, snap.plane)`, which
            // pairs the obstacle handle with the carrier's already-
            // resolved top-plane (avoiding a redundant lookup).
            elem.set_sector(snap.sector);
            elem.set_material(snap.material);
            // Pin the carried's display_order just in front of the
            // carrier every frame (lift/wait/walk/drop) so the two
            // sprites stay stacked correctly when other entities cross
            // the draw list.
            let sprite = &mut elem.sprite;
            sprite.display_order_ref = Some(snap.carrier_id);
            sprite.behind_display_order_ref = false;
        }

        // Update the carried entity's `PositionInterface` with the
        // carrier's obstacle + plane + material and the new map
        // position, then reproject.  This ensures the sprite's 3D
        // projection lands on the correct plane the same frame the
        // carrier crosses an elevation line or sector boundary —
        // otherwise the corpse is one frame late.
        {
            let pi = target.position_iface_mut();
            pi.set_obstacle(snap.obstacle_index, snap.plane);
            pi.set_material(snap.material);
            pi.set_position_map(crate::geo2d::pt(snap.pos.x, snap.pos.y));
        }

        // ── Climb-on-shoulders branch ──────────────────────────
        // Animation sync direction is *inverted* compared to corpse-
        // carry: the climber drives `ClimbingUpOnShoulders` from
        // `tick_abilities`, and the helper syncs onto it via
        // `TransitionHelpingClimbingUp`.  Once the climb finishes both
        // PCs sit on posture-driven idle poses (climber →
        // WaitingOnShoulders, helper → WaitingCarryingOnShoulders);
        // during the helper's `WalkingCarryingOnShoulders` we force
        // `WaitingOnShoulders` on the climber.
        if on_shoulders {
            let carried_anim = match snap.carrier_last_action {
                // Helper walking with PC on shoulders — climber rides idle.
                OrderType::WalkingCarryingOnShoulders => Some(OrderType::WaitingOnShoulders),
                // Climber's own active climb anim is driven by tick_abilities;
                // do NOT override it here.  Anything else (idle helper) →
                // climber settles into WaitingOnShoulders synced to helper.
                _ if matches!(
                    snap.target_last_action,
                    OrderType::ClimbingUpOnShoulders | OrderType::ClimbingDownFromShoulders
                ) =>
                {
                    None
                }
                _ => Some(OrderType::WaitingOnShoulders),
            };
            if let Some(anim) = carried_anim {
                let sprite = &mut target.element_data_mut().sprite;
                let is_walking = matches!(
                    snap.carrier_last_action,
                    OrderType::WalkingCarryingOnShoulders
                );
                if is_walking {
                    sprite.force_animation(anim, carried_dir_u16);
                } else {
                    sprite.force_sprite_row(anim, carried_dir_u16);
                    sprite.synchronize_anim(snap.carrier_frame, snap.carrier_frame_count);
                }
            }

            // The `target` borrow is no longer used past this point, so
            // NLL releases the `entities` borrow and the helper lookup
            // below can take a fresh `&mut entities[carrier_id]`.
            //
            // While the climber plays `ClimbingUpOnShoulders` /
            // `ClimbingDownFromShoulders`, force the helper to the
            // matching `TransitionHelpingClimbing*` row synchronized to
            // the climber's frame.
            let helper_anim = match snap.target_last_action {
                OrderType::ClimbingUpOnShoulders => Some(OrderType::TransitionHelpingClimbingUp),
                OrderType::ClimbingDownFromShoulders => {
                    Some(OrderType::TransitionHelpingClimbingDown)
                }
                _ => None,
            };
            if let Some(anim) = helper_anim
                && let Some(Some(helper)) = entities.get_mut(snap.carrier_id.0 as usize)
            {
                let helper_dir = u16::try_from(helper.element_data().direction()).unwrap_or(0);
                let sprite = &mut helper.element_data_mut().sprite;
                sprite.force_sprite_row(anim, helper_dir);
                sprite.synchronize_anim(snap.target_frame, snap.target_frame_count);
            }
            continue;
        }

        // ── Corpse-carry branch (default) ──────────────────────
        // Pick the carried animation based on the carrier's current
        // phase and carry style.
        let carried_anim = match snap.carrier_last_action {
            // Lifting the corpse — synced with carrier's lift anim.
            OrderType::TransitionWaitingUprightCarryingCorpse => {
                if snap.little_john_style {
                    Some(OrderType::BeingLiftedLittleJohn)
                } else {
                    Some(OrderType::BeingLiftedPeasantC)
                }
            }
            // Dropping the corpse — synced with carrier's drop anim.
            OrderType::TransitionCarryingCorpseWaitingUpright => {
                if snap.little_john_style {
                    Some(OrderType::BeingDroppedLittleJohn)
                } else {
                    Some(OrderType::BeingDroppedPeasantC)
                }
            }
            // Walking with corpse — forced animation (frame reset
            // with direction); we use ForceAnimation rather than
            // SynchronizeAnim so the frame starts at 0.
            OrderType::WalkingWithCorpse => {
                if snap.little_john_style {
                    Some(OrderType::BeingCarriedLittleJohn)
                } else {
                    Some(OrderType::BeingCarriedPeasantC)
                }
            }
            // Waiting with corpse (default / idle carry pose) — synced
            // with carrier's sprite.
            _ => {
                if snap.little_john_style {
                    Some(OrderType::BeingCarriedLittleJohn)
                } else {
                    Some(OrderType::BeingCarriedPeasantC)
                }
            }
        };

        if let Some(anim) = carried_anim {
            let sprite = &mut target.element_data_mut().sprite;
            // For WalkingWithCorpse use `force_animation` which resets
            // frame/frame_count to 0.  Everything else forces the
            // sprite row and then syncs the frame with the carrier.
            let is_walking = matches!(snap.carrier_last_action, OrderType::WalkingWithCorpse);
            if is_walking {
                sprite.force_animation(anim, carried_dir_u16);
            } else {
                sprite.force_sprite_row(anim, carried_dir_u16);
                sprite.synchronize_anim(snap.carrier_frame, snap.carrier_frame_count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_interface::Point3D;
    use crate::sight_obstacle::ObstacleList;

    #[test]
    fn can_carry_on_shoulders_clear_with_no_obstacles() {
        // No obstacles anywhere → ceiling column is always clear.
        let list = ObstacleList {
            static_obstacles: &[],
            dynamic_obstacles: &[],
            static_active: &[],
        };
        let pos = Point3D {
            x: 100.0,
            y: 100.0,
            z: 0.0,
        };
        assert!(can_carry_on_shoulders(pos, list));
    }
}
