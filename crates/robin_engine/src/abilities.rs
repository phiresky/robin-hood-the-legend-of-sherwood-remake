//! Hero special abilities — carry, tie-up, heal, whistle, listen, trap placement.
//!
//! Each ability follows the [`crate::bow_shot`] pattern:
//!
//! 1. A `begin_*` function is called when the engine dispatches a
//!    `Command::*` sequence element to an actor.  It validates the actor
//!    and target, pushes an animation order, and sets
//!    [`ActiveAbility`][crate::movement::ActiveAbility] on the actor.
//!
//! 2. [`tick_ability`] runs from the selected actor's legacy owner slot and
//!    drives its sprite via `perform_action`. `Done` emits the one-shot
//!    effect; `Terminated` releases the selected sequence element.
//!
//! 3. The engine applies cross-entity effects (posture changes, HP
//!    restoration, etc.) from the returned [`AbilityTickResult`] values
//!    *after* the mutable entity borrow is released.

use crate::coordinates::MapPoint;
use crate::element::{
    ActionState, Command, Entity, EntityId, GameMaterial, ListenPhase, Posture, ReceivePursePhase,
};
use crate::entities::Entities;
use crate::movement::{AbilityKind, ActiveAbility};
use crate::order::{Order, OrderType};
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
    carrier_position: crate::coordinates::WorldPoint3D,
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
    /// A previously-Done selected ability reached `RHMOTION_TERMINATED` and
    /// may now release its driving sequence element.
    Terminated {
        actor_id: EntityId,
        kind: AbilityKind,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    Aborted {
        actor_id: EntityId,
        kind: AbilityKind,
        seq_id: SequenceId,
        elem_idx: usize,
        order_id: Option<NonZeroU32>,
    },
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
        carrier_pos: MapPoint,
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
        position: MapPoint,
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
        target_pos: MapPoint,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// Stuteley finished the wasp nest throw animation.
    ThrowWaspNestDone {
        actor_id: EntityId,
        target_pos: MapPoint,
        seq_id: SequenceId,
        elem_idx: usize,
    },
    /// A PC finished the purse-throw animation.  The engine handler
    /// spawns the purse projectile; its impact handler scatters coins.
    ThrowPurseDone {
        actor_id: EntityId,
        /// 2D ground target the purse arcs toward.
        target_pos: MapPoint,
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
    StrangleSetupDone {
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
    /// helper) — the first Execute of the climbing order did the latch so
    /// that pairing exists while the animation runs.  The handler terminates
    /// the driving sequence element and parks the helper on a low-priority Wait
    /// so its AI re-enters the idle loop while frozen-on-shoulders.
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
/// - **Building hulk**: the re-select + hulk start step when picking
///   up in a building sector is applied in the `Command::TakeCorpse`
///   handler in `engine/tick.rs` after `begin_carry` succeeds.
pub fn begin_carry(
    entities: &mut Entities,
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

    // Validate the same target-side invariants as
    // RHElementActorPC::CheckSequenceElementValidity(TAKE_CORPSE). A body
    // already linked to this PC remains valid: authentic restored states can
    // retain that self-link while the PC is upright, and Original explicitly
    // rejects only a *different* carrier (RHelementactorpc.cpp:6884-6909).
    let target_valid = match entities.get(target_id) {
        Some(e) => {
            if !e.is_human() {
                false
            } else {
                let posture = e.element_data().posture;
                let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                let dead = e.is_dead();
                let available_carrier = e
                    .human_data()
                    .is_some_and(|human| human.carrier.is_none_or(|carrier| carrier == carrier_id));
                e.element_data().active
                    && (unconscious || dead)
                    && matches!(
                        posture,
                        Posture::Lying | Posture::Dead | Posture::DeadBack | Posture::Tied
                    )
                    && available_carrier
            }
        }
        None => false,
    };
    if !target_valid {
        return BeginResult::Impossible;
    }

    let target_pos = {
        let target = entities[target_id].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate only the carrier invariants checked by Original. In particular,
    // RHElementActorPC::CheckSequenceElementValidity(TAKE_CORPSE) never reads
    // `mpCarried`: an authentic restored PC may still be linked to a different
    // carried body while a new TakeCorpse is translated. Translate authors the
    // pickup order anyway, and the first Execute replaces `mpCarried`.
    let carrier = match entities.get_mut(carrier_id) {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !carrier.is_pc() || carrier.is_dead() {
        return BeginResult::Impossible;
    }
    let order_id = alloc_order_id(order_id_counter);

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
        done_effect_applied: false,
        strangle_initialized: false,
    };
    actor.clear_path();

    let mut order = Order::new(
        OrderType::TransitionWaitingUprightCarryingCorpse,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Unlike the strangle/heal/tie ability inits, the corpse-carry
    // transition does not turn the carrier toward the corpse: the
    // carrier keeps the facing it arrived with, and the carried body is
    // aligned relative to that direction instead.

    BeginResult::Started
}

/// Publish the corpse/carrier relationship at the pickup order's first
/// Execute boundary.
///
/// Original performs these assignments inside
/// `RHANIMATION_TRANSITION_WAITING_UPRIGHT_CARRYING_CORPSE`, immediately
/// before freezing and positioning the body (RHelementactorpc.cpp:4842-4864),
/// not when the command is translated.
pub(crate) fn initialize_carry_relationship(
    entities: &mut Entities,
    carrier_id: EntityId,
    target_id: EntityId,
) {
    // Original snapshots the new body's posture only when the pickup order
    // first executes. Translation may still be playing a generated drop
    // prefix for an older restored body, whose own mCarriedPosture must remain
    // authoritative until that prefix releases it.
    let target_posture = entities
        .get(target_id)
        .unwrap_or_else(|| panic!("Carry target {target_id:?} vanished at initialization"))
        .element_data()
        .posture;
    let target_posture = if target_posture == Posture::Dead {
        Posture::DeadBack
    } else {
        target_posture
    };
    let carrier = entities
        .get_mut(carrier_id)
        .unwrap_or_else(|| panic!("Carry owner {carrier_id:?} vanished at initialization"));
    let pc = carrier
        .pc_data_mut()
        .unwrap_or_else(|| panic!("Carry owner {carrier_id:?} is not a PC"));
    // Original assigns mpCarried unconditionally. This intentionally replaces
    // a stale/restored link to another body after the Execute-time validity
    // check has accepted the new target.
    let acquiring_target = pc.carried != Some(target_id);
    pc.carried = Some(target_id);
    if acquiring_target {
        // This helper may be reached again while the pickup animation remains
        // active. Original's assignment is guarded by IsInitialisation(), so
        // do not resnapshot after CarryDone changes the target to Carried.
        pc.set_live_carried_posture(target_posture);
    }

    let target = entities
        .get_mut(target_id)
        .unwrap_or_else(|| panic!("Carry target {target_id:?} vanished at initialization"));
    let human = target
        .human_data_mut()
        .unwrap_or_else(|| panic!("Carry target {target_id:?} is not human"));
    human.carrier = Some(carrier_id);
    target
        .actor_data_mut()
        .unwrap_or_else(|| panic!("Carry target {target_id:?} is not an actor"))
        .is_ignored_for_anti_collision = false;
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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    carrier_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let carrier = match entities.get_mut(carrier_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
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
/// check and order creation happen while the interaction is instructed.
/// Posture/link/position setup is deferred to the climbing order's first
/// Execute, matching `RHANIMATION_CLIMBING_UP_ON_SHOULDERS` initialization.
///
/// The order is pushed on the *climber*'s sequence element.  The first
/// Execute later calls [`initialize_climb_on_shoulders_relationship`].
#[allow(clippy::too_many_arguments)]
pub fn begin_climb_on_shoulders(
    entities: &mut Entities,
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
    let (helper_pos_map, helper_pos_3d, helper_valid) = match entities.get(helper_id) {
        Some(e) => {
            let valid =
                e.is_pc() && !e.is_dead() && e.element_data().posture == Posture::HelpingToClimb;
            (
                e.element_data().position_map(),
                e.position_iface().get_position(),
                valid,
            )
        }
        None => (MapPoint { x: 0.0, y: 0.0 }, Default::default(), false),
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
    let climber = match entities.get_mut(climber_id) {
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
    let order_id = alloc_order_id(order_id_counter);
    let actor = climber.actor_data_mut().unwrap();
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ClimbOnShoulders),
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(helper_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    // Push the climbing animation order on the climber's sequence
    // element. Direction and the carrier relationship are initialized by
    // the order's first Execute, not by this translation/instruction pass.
    let mut order = Order::new(
        OrderType::ClimbingUpOnShoulders,
        helper_pos_map.x,
        helper_pos_map.y,
        order_id,
    );
    order.target_actor = Some(helper_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    ClimbResult::Started
}

/// Apply the one-shot relationship setup performed by Original when the
/// `ClimbingUpOnShoulders` order first executes.
///
/// `RHElementActorPC::Translate` only appends the order. Its Execute arm later
/// links the pair, aims the helper from the climber's pre-snap position, flips
/// both postures, and finally snaps the climber onto the helper. Keeping that
/// ordering matters: sampling the climber after the snap collapses the facing
/// vector to zero.
pub(crate) fn initialize_climb_on_shoulders_relationship(
    entities: &mut Entities,
    climber_id: EntityId,
    helper_id: EntityId,
) {
    let climber_pos = entities
        .get(climber_id)
        .unwrap_or_else(|| panic!("climb-on-shoulders climber {climber_id:?} disappeared"))
        .element_data()
        .position_map();
    let helper_pos = entities
        .get(helper_id)
        .unwrap_or_else(|| panic!("climb-on-shoulders helper {helper_id:?} disappeared"))
        .element_data()
        .position_map();
    let helper_facing = crate::position_interface::vector_to_sector_0_to_15_iso(
        climber_pos.x - helper_pos.x,
        climber_pos.y - helper_pos.y,
    );

    {
        let helper = entities
            .get_mut(helper_id)
            .expect("validated climb-on-shoulders helper disappeared during initialization");
        let pc = helper
            .pc_data_mut()
            .expect("climb-on-shoulders helper is not a PC");
        pc.carried = Some(climber_id);
        pc.set_live_carried_posture(Posture::OnShoulders);
    }
    entities
        .get_mut(climber_id)
        .expect("validated climb-on-shoulders climber disappeared during initialization")
        .human_data_mut()
        .expect("climb-on-shoulders climber is not human")
        .carrier = Some(helper_id);
    entities
        .get_mut(helper_id)
        .expect("validated climb-on-shoulders helper disappeared before facing setup")
        .element_data_mut()
        .set_direction_goal(helper_facing);
    {
        let climber = entities
            .get_mut(climber_id)
            .expect("validated climb-on-shoulders climber disappeared before posture setup");
        climber.set_posture(Posture::OnShoulders);
        climber
            .actor_data_mut()
            .expect("climb-on-shoulders climber lost actor state")
            .action_state = ActionState::Waiting;
    }
    {
        let helper = entities
            .get_mut(helper_id)
            .expect("validated climb-on-shoulders helper disappeared before posture setup");
        helper.set_posture(Posture::CarryingOnShoulders);
        helper
            .actor_data_mut()
            .expect("climb-on-shoulders helper lost actor state")
            .action_state = ActionState::Waiting;
    }
    entities
        .get_mut(climber_id)
        .expect("validated climb-on-shoulders climber disappeared before snap")
        .element_data_mut()
        .set_position_map(helper_pos);
}

/// Start the dismount animation for a PC currently `OnShoulders`.
///
/// Called when `Command::ClimbDownFromShoulders` is dispatched.
///
/// The order is pushed on the *climber*'s sequence element.  The helper
/// (carrier) is identified via the climber's `human.carrier`
/// back-reference latched by [`initialize_climb_on_shoulders_relationship`].
/// Posture reset, carrier-link severance and landing-position resolution
/// happen in the [`AbilityTickResult::ClimbDownFromShouldersDone`] consumer
/// after the animation reaches its terminated state.
pub fn begin_climb_down_from_shoulders(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    climber_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    // Validate climber: must be a living PC currently OnShoulders with
    // a carrier reference.
    let carrier_id = match entities.get(climber_id) {
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
    let climber = match entities.get_mut(climber_id) {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    let actor = climber.actor_data_mut().unwrap();
    actor.active_ability = ActiveAbility {
        kind: Some(AbilityKind::ClimbDownFromShoulders),
        done_effect_applied: false,
        strangle_initialized: false,
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
    order.target_actor = Some(carrier_id.index());
    order.compute_direction = false;

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
pub fn begin_tie(
    entities: &mut Entities,
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
    let target_valid = match entities.get(target_id) {
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
        let target = entities[target_id].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate actor: must be alive, human, not already busy.
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };

    let mut order = Order::new(OrderType::Tying, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    healer_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    // `RHElementActorPC::Translate(RHCOMMAND_HEAL)` only authors the
    // Healing order.  The living/injured/range predicate belongs to the
    // following Execute initialization (`RHelementactorpc.cpp:3055-3062,
    // 4524-4532`), after the element has become selected.  In particular, a
    // post-seek Heal whose victim recovered while the healer was travelling
    // is still visible as Healing for this manager phase and terminates from
    // Execute on the next actor phase.  Do not prevalidate hit points here.
    //
    // Retain the command's structural target invariant: supported targets
    // are PCs and FX targets.  Missing/unsupported interaction targets are
    // not valid authored Heal commands.
    let target_valid = match entities.get(target_id) {
        Some(e) => e.kind().is_fx_target() || e.is_pc(),
        None => false,
    };
    if !target_valid {
        return BeginResult::Impossible;
    }

    let target_pos = {
        let target = entities[target_id].as_ref().unwrap();
        target.element_data().position_map()
    };

    // Validate healer: must be alive PC.
    let healer = match entities.get_mut(healer_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    // `RHElementActorPC::Translate(RHCOMMAND_HEAL)` queues the healing order
    // without stopping the actor or rewriting its logical action state.

    // Queue the canonical Healing order; owner-local ability dispatch swaps to
    // `OrderType::Eating` when the target is the healer itself.
    let mut order = Order::new(OrderType::Healing, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Whistle (Robin Hood)
// ═══════════════════════════════════════════════════════════════════

/// Start whistling to attract guards.
///
/// Called when `Command::WhistleCmd` is dispatched.
///
/// Arms `whistle_wait_time = TIME_LISTEN_WAIT` (25) so owner-local dispatch
/// can decrement it each frame and `render_listen_ping` can draw the
/// expanding noise ellipse during the final `TIME_LISTEN` (5) frames
/// (the Whistling arm of the shared Listen/Whistle ellipse render).
pub fn begin_whistle(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    // Original `RHElementActorPC::Translate(RHCOMMAND_WHISTLE)` only queues
    // the whistling order.  In particular, it does not call `Stop()` or
    // overwrite the actor's logical action state: a PC that was bored or
    // moving keeps that state until the animation reaches its terminal
    // `SetStates` boundary.  `mulWaitTime` is the actor's single serialized
    // countdown, so keep the renderer-only mirror synchronized with it.
    actor.wait_time = TIME_LISTEN_WAIT;
    actor.seek_refresh_wait = TIME_LISTEN_WAIT;
    actor.whistle_wait_time = TIME_LISTEN_WAIT;

    let mut order = Order::new(OrderType::Whistling, 0.0, 0.0, order_id);
    order.compute_direction = false;

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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    // `RHElementActorPC::Translate(RHCOMMAND_EAT)` only appends the eating
    // order.  The current path and action state remain authoritative until
    // `RHANIMATION_EATING` terminates and calls `SetStates`.

    let mut order = Order::new(OrderType::Eating, 0.0, 0.0, order_id);
    order.compute_direction = false;

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
/// [`tick_ability`] emits [`AbilityTickResult::HitDone`], and the
/// engine handler (`engine::combat`) launches a
/// [`Command::ReceiveHitDamage`] damage element on the target with
/// concussion 80 (`Action::Hit`) or 150 (`Action::HitHard`) based on
/// whether the attacker's profile carries the HitHard action slot.
///
/// [`Command::ReceiveHitDamage`]: crate::element::Command::ReceiveHitDamage
pub fn begin_hit(
    entities: &mut Entities,
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
    //
    // Liveness is deliberately NOT gated here. `RHElementActorHuman::Instruct`
    // (`original-code/RHelementactorhuman.cpp:2098-2117`) inserts the HITTING
    // order for `RHCOMMAND_HIT` unconditionally — its only branch is the
    // `Think(EVENT_STOP)` poke for a moving NPC antagonist. A dead, tied,
    // netted or carried victim is rejected later, by the HITTING
    // initialisation gate `CheckSequenceElementValidity( ..., true )`
    // (`RHelementactorhuman.cpp:4583-4590`, whose HIT arm tests
    // `IsOutOfOrder()` at `RHelementactorhuman.cpp:6771-6816`). Rust runs that
    // same gate in `EngineInner::tick_pending_hit_init`
    // (`engine/combat.rs`), and the Original's actor is visibly committed to
    // `RHCOMMAND_HIT` — including the walk→wait transition Instruct splices in
    // ahead of HITTING — for the frames before that abort.
    let target_pos = match entities.get(target_id) {
        Some(e) if e.is_human() => e.element_data().position_map(),
        _ => return BeginResult::Impossible,
    };

    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };

    let mut order = Order::new(OrderType::Hitting, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

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
/// [`tick_ability`] emits [`AbilityTickResult::StrangleDone`], and
/// the engine handler launches a full-life-points
/// [`Command::ReceiveDamage`] element that kills the victim.
///
/// [`Command::ReceiveDamage`]: crate::element::Command::ReceiveDamage
pub fn begin_strangle(
    entities: &mut Entities,
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

    // PC::Translate(STRANGLE) appends the order without revalidating the
    // antagonist. A retained post-seek command can therefore become visible
    // for one frame after its target dies; the first Strangling Execute runs
    // CheckSequenceElementValidity and aborts it on the following owner slot.
    // Keep only the type constraint needed by that Execute arm here.
    let target = entities
        .get(target_id)
        .unwrap_or_else(|| panic!("validated strangle target {target_id:?} vanished before begin"));
    if !target.is_human() || target.is_pc() {
        return BeginResult::Impossible;
    }
    target.actor_data().unwrap_or_else(|| {
        panic!("validated strangle target {target_id:?} is an NPC human without actor state")
    });
    target.ai_controller().unwrap_or_else(|| {
        panic!("validated strangle target {target_id:?} is an NPC human without AI state")
    });
    let target_pos = target.element_data().position_map();

    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    // PC::Translate(RHCOMMAND_STRANGLE) only appends the Strangling
    // order.  In particular, a seek that launches its post-seek sequence
    // synchronously retains the movement state until the later posture
    // transition actually changes it.

    let mut order = Order::new(OrderType::Strangling, target_pos.x, target_pos.y, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

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
/// `active_ability` / owner-local [`tick_ability`] machinery:
///
/// 1. `begin_listen` sets `ListenPhase::EnterTransition` and starts
///    the entry transition animation.
/// 2. `tick_ability` drives the transition sprite; on `Done` it
///    flips `action_state = Listening`, `ListenPhase::CountingDown`,
///    and returns an `AbilityTickResult::ListenEntered` so the
///    engine can send a `SelectAction(Listen)` PC message.
/// 3. the selected PC owner arm arms `listen_wait_time`, decrements
///    it each frame, fires the one-shot reveal + FX-target `Heard()`
///    when it reaches 0, and advances to `ListenPhase::ExitTransition`.
/// 4. `tick_ability` drives the exit transition sprite; on `Done`
///    it cleans up and returns `AbilityTickResult::ListenDone` so
///    the engine sends `UnselectAction` and terminates the driving
///    sequence element.
pub fn begin_listen(
    entities: &mut Entities,
    profiles: &crate::profiles::ProfileManager,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    // Original `Translate(RHCOMMAND_ENTER_LISTEN)` preserves the current
    // path/action state while queuing all three orders and writes the shared
    // serialized `mulWaitTime` immediately.  The transition animation owns
    // the sprite independently of that logical state.
    actor.wait_time = TIME_LISTEN_WAIT;
    actor.seek_refresh_wait = TIME_LISTEN_WAIT;
    actor.listen_phase = ListenPhase::EnterTransition;
    actor.listen_wait_time = 0;

    let mut order = Order::new(
        OrderType::TransitionWaitingUprightListening,
        0.0,
        0.0,
        order_id,
    );
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);
    for order_type in [
        OrderType::Listening,
        OrderType::TransitionListeningWaitingUpright,
    ] {
        let mut order = Order::new(order_type, 0.0, 0.0, alloc_order_id(order_id_counter));
        order.compute_direction = false;

        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }

    BeginResult::Started
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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: MapPoint,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None, // ground target, not entity
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::ThrowingNet, target_pos.x, target_pos.y, order_id);
    order.compute_direction = false;

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
    entities: &mut Entities,
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
    entities: &mut Entities,
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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
    kind: AbilityKind,
    order_type: OrderType,
) -> BeginResult {
    let target_pos = match entities.get(target_id) {
        Some(e) => e.element_data().position_map(),
        None => return BeginResult::Impossible,
    };
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(order_type, target_pos.x, target_pos.y, order_id);
    order.compute_direction = false;

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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: MapPoint,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
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
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: MapPoint,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_id),
    };
    actor.clear_path();
    // `THROW_PURSE` requires Waiting, but Original's action transition owns
    // that state change.  In particular, a Bored actor remains Bored while
    // `WAITING_UPRIGHT_BORED_WAITING_UPRIGHT` is playing and becomes Waiting
    // only when that prefix completes.
    // TODO(original-parity): audit the equivalent eager Waiting writes in
    // the sibling throw/pay begin paths before changing their behavior.

    let mut order = Order::new(
        OrderType::ThrowingPurse,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.compute_direction = false;

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
/// Installs the Paying order without changing the PC's facing. Original
/// translation only constructs the order; the live validity check, facing
/// change, and "give money" speech belong to the order's first Execute.
/// On completion, [`AbilityTickResult::PayDone`] deducts the beggar salary
/// and launches a `ReceivePurse` sequence element on the beggar.
pub fn begin_pay(
    entities: &mut Entities,
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
    let beggar_valid = match entities.get(beggar_id) {
        Some(e @ Entity::Civilian(c)) => {
            !e.is_dead()
                && !c.human.unconscious
                && c.npc.attached_scroll.is_none()
                && c.civilian.beggar_scroll_sets.is_some()
        }
        _ => false,
    };
    if !beggar_valid {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let pc_entity = match entities.get_mut(pc_id) {
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(beggar_id),
        order_id: Some(order_id),
    };
    actor.clear_path();
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Paying, pc_pos.x, pc_pos.y, order_id);
    order.target_actor = Some(beggar_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

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
/// owner-local dispatch can fire [`EngineInner::reveal_scrolls`]
/// on the Waiting→Transition boundary.
///
/// [`EngineInner::reveal_scrolls`]: crate::engine::EngineInner::reveal_scrolls
pub fn begin_receive_purse(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    beggar_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let beggar = match entities.get_mut(beggar_id) {
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

    let order_ids = [
        alloc_order_id(order_id_counter),
        alloc_order_id(order_id_counter),
        alloc_order_id(order_id_counter),
    ];
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
        done_effect_applied: false,
        strangle_initialized: false,
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: None,
        order_id: Some(order_ids[0]),
    };
    actor.clear_path();
    actor.receive_purse_phase = ReceivePursePhase::Receiving;

    for (order_type, order_id) in [
        OrderType::ReceivingPurse,
        OrderType::WaitingWithPurse,
        OrderType::TransitionWaitingWithPurseWaitingUpright,
    ]
    .into_iter()
    .zip(order_ids)
    {
        let mut order = Order::new(order_type, 0.0, 0.0, order_id);
        order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame ability tick
// ═══════════════════════════════════════════════════════════════════

/// Map [`AbilityKind`] to the [`OrderType`] that drives its animation.
pub(crate) fn ability_order_type(kind: AbilityKind) -> OrderType {
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
            "{kind:?} is handled inline in tick_ability — \
             ability_order_type should never be called for it"
        ),
    }
}

/// Rebuild Rust's derived ability owner latch after loading an Original save.
///
/// Original does not serialize a second "active ability" object: the selected
/// in-progress sequence element and its current `RHOrder` are the authority.
/// Rust normally creates this latch in `begin_*`, but a loaded sequence has
/// already crossed that boundary. Reconstruct it from the same authoritative
/// order so completion effects continue on the correct frame.
pub(crate) fn restore_loaded_active_abilities(
    entities: &mut Entities,
    sequence_manager: &SequenceManager,
) {
    fn classify_loaded_ability_order(
        element: &crate::sequence::SequenceElement,
        order: &Order,
    ) -> Option<(AbilityKind, Option<ListenPhase>, Option<ReceivePursePhase>)> {
        Some(match order.order_type {
            OrderType::TransitionWaitingUprightCarryingCorpse => (AbilityKind::Carry, None, None),
            OrderType::TransitionCarryingCorpseWaitingUpright => (AbilityKind::Drop, None, None),
            OrderType::Tying => (AbilityKind::Tie, None, None),
            OrderType::Healing | OrderType::Eating if element.command == Command::HealCmd => {
                (AbilityKind::Heal, None, None)
            }
            OrderType::Whistling => (AbilityKind::Whistle, None, None),
            OrderType::TransitionWaitingUprightListening => (
                AbilityKind::Listen,
                Some(ListenPhase::EnterTransition),
                None,
            ),
            OrderType::Listening => (AbilityKind::Listen, Some(ListenPhase::CountingDown), None),
            OrderType::TransitionListeningWaitingUpright => {
                (AbilityKind::Listen, Some(ListenPhase::ExitTransition), None)
            }
            OrderType::ThrowingNet => (AbilityKind::ThrowNet, None, None),
            OrderType::ThrowingWaspNest => (AbilityKind::ThrowWaspNest, None, None),
            OrderType::ThrowingPurse => (AbilityKind::ThrowPurse, None, None),
            OrderType::ThrowingApple => (AbilityKind::ThrowApple, None, None),
            OrderType::ThrowingStone => (AbilityKind::ThrowStone, None, None),
            OrderType::Paying => (AbilityKind::Pay, None, None),
            OrderType::ReceivingPurse => (
                AbilityKind::ReceivePurse,
                None,
                Some(ReceivePursePhase::Receiving),
            ),
            OrderType::WaitingWithPurse => (
                AbilityKind::ReceivePurse,
                None,
                Some(ReceivePursePhase::Waiting),
            ),
            OrderType::TransitionWaitingWithPurseWaitingUpright => (
                AbilityKind::ReceivePurse,
                None,
                Some(ReceivePursePhase::Transition),
            ),
            OrderType::Hitting => (AbilityKind::Hit, None, None),
            OrderType::Strangling => (AbilityKind::Strangle, None, None),
            OrderType::Eating => (AbilityKind::Eat, None, None),
            OrderType::ClimbingUpOnShoulders => (AbilityKind::ClimbOnShoulders, None, None),
            OrderType::ClimbingDownFromShoulders => {
                (AbilityKind::ClimbDownFromShoulders, None, None)
            }
            _ => return None,
        })
    }

    let active = sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .enumerate()
                .filter_map(move |(element_index, element)| {
                    let owner = element.owner?;
                    if element.state != crate::sequence::SequenceState::InProgress {
                        return None;
                    }
                    let current_order = element.current_order()?;
                    let (order, (kind, listen_phase, receive_purse_phase)) =
                        classify_loaded_ability_order(element, current_order)
                            .map(|classification| (current_order, classification))
                            .or_else(|| {
                                // Original actor ownership remains on the
                                // sequence element while its launch-time
                                // transition prefix plays. The Rust-only
                                // ability latch must therefore target the
                                // first non-transition order already stored
                                // in that same serialized element. Using the
                                // saved remaining-transition count preserves
                                // the exact queue boundary without guessing
                                // from animation names or save provenance.
                                let ability_order =
                                    element.orders.get(element.num_transition_orders)?;
                                classify_loaded_ability_order(element, ability_order)
                                    .map(|classification| (ability_order, classification))
                            })?;
                    Some((
                        owner,
                        ActiveAbility {
                            kind: Some(kind),
                            sequence_id: Some(sequence.id),
                            element_index,
                            target: order.antagonist,
                            order_id: Some(order.order_id),
                            // A current serialized order has not yet been
                            // removed at its DONE boundary. Its world-side
                            // effects, if any, are already authoritative in
                            // the save and must not be applied twice.
                            done_effect_applied: order.done,
                            // Execute-time strangle initialization precedes
                            // installation of the Strangling order.
                            strangle_initialized: kind == AbilityKind::Strangle,
                        },
                        listen_phase,
                        receive_purse_phase,
                    ))
                })
        })
        .collect::<Vec<_>>();

    for (owner, mut ability, listen_phase, receive_purse_phase) in active {
        let entity = entities
            .get_mut(owner)
            .unwrap_or_else(|| panic!("loaded ability owner {owner:?} disappeared"));
        if ability.kind == Some(AbilityKind::Drop) && ability.target.is_none() {
            ability.target = entity
                .pc_data()
                .unwrap_or_else(|| panic!("loaded Drop owner {owner:?} is not a PC"))
                .carried;
            assert!(
                ability.target.is_some(),
                "loaded Drop owner {owner:?} has neither an order antagonist nor a carried body"
            );
        }
        let actor = entity
            .actor_data_mut()
            .unwrap_or_else(|| panic!("loaded ability owner {owner:?} is not an actor"));
        actor.active_ability = ability;
        if let Some(phase) = listen_phase {
            actor.listen_phase = phase;
        }
        if let Some(phase) = receive_purse_phase {
            actor.receive_purse_phase = phase;
        }
    }
}

/// Advance the active ability for one actor.
///
/// This is the per-owner unit used by the engine's creation-ordered element
/// pass.
pub fn tick_ability(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sequence_manager: &SequenceManager,
    requested_actor: EntityId,
    sprite_frozen: bool,
) -> Vec<AbilityTickResult> {
    let mut results = Vec::new();
    let entity = entities
        .get(requested_actor)
        .unwrap_or_else(|| panic!("ability owner {requested_actor:?} disappeared"));
    assert!(
        entity.actor_data().is_some(),
        "ability owner {requested_actor:?} is not an actor"
    );
    let entity_id = requested_actor;
    let actor = match entity.actor_data() {
        Some(a) => a,
        None => unreachable!("ability owner invariant checked above"),
    };
    if !actor.active_ability.is_active() {
        return results;
    }

    let ability = actor.active_ability.clone();
    let listen_phase = actor.listen_phase;
    let receive_purse_phase = actor.receive_purse_phase;
    let kind = ability.kind.unwrap(); // safe: is_active() checked

    // RHElementActorPC::Execute(TYING) revalidates the antagonist every
    // frame. The DONE callback itself changes Lying -> Tied, so the next
    // Execute deliberately fails this check and aborts/releases the Tie
    // element instead of playing the unused animation tail.
    if kind == AbilityKind::Tie {
        let target_id = ability
            .target
            .expect("active Tie ability must retain its antagonist");
        let target_valid = entities.get(target_id).is_some_and(|target| {
            target.human_data().is_some_and(|human| human.unconscious)
                && target.element_data().posture == Posture::Lying
        });
        if !target_valid {
            results.push(AbilityTickResult::Aborted {
                actor_id: entity_id,
                kind,
                seq_id: ability.sequence_id.expect("Tie ability sequence"),
                elem_idx: ability.element_index,
                order_id: ability.order_id,
            });
            return results;
        }
    }

    // RHElementActorPC::Perform(STRANGLING) uses C++ `&&` ordering:
    // attacker TurnFast runs first, and the victim is not advanced until a
    // later tick where the attacker was already aligned. PerformAction is
    // likewise deferred until both calls return false. Direction goals and
    // the victim FREEZE lock are installed by the engine at the original
    // post-translation initialization boundary.
    if kind == AbilityKind::Strangle {
        let victim_id = ability
            .target
            .expect("active Strangle ability must retain its antagonist");
        if entities
            .get_mut(requested_actor)
            .expect("validated strangle owner vanished before TurnFast")
            .position_iface_mut()
            .turn_fast()
        {
            advance_pre_action_strangle_victim_if_due(sim, entities, requested_actor, victim_id);
            return results;
        }
        let victim = entities
            .get_mut(victim_id)
            .unwrap_or_else(|| panic!("strangle victim {victim_id:?} vanished while turning"));
        assert!(
            victim.actor_data().is_some(),
            "strangle victim {victim_id:?} lost required actor state while turning"
        );
        if victim.position_iface_mut().turn_fast() {
            advance_pre_action_strangle_victim_if_due(sim, entities, requested_actor, victim_id);
            return results;
        }
    }

    if kind == AbilityKind::ClimbOnShoulders {
        let helper_id = ability
            .target
            .expect("active ClimbOnShoulders ability must retain its helper");
        let helper_direction = entities
            .get(helper_id)
            .unwrap_or_else(|| {
                panic!("climb-on-shoulders helper {helper_id:?} vanished during Execute")
            })
            .element_data()
            .direction();
        // Original reissues this progressive facing goal before Turn on every
        // Execute of RHANIMATION_CLIMBING_UP_ON_SHOULDERS.
        entities
            .get_mut(requested_actor)
            .expect("climb-on-shoulders owner vanished before facing update")
            .element_data_mut()
            .set_direction_goal((helper_direction + 8) & 15);
    }

    if kind == AbilityKind::Carry && !sprite_frozen {
        let order_id = ability.order_id.expect("active Carry ability order");
        let target_id = ability.target.expect("active Carry ability target");
        initialize_carry_relationship(entities, requested_actor, target_id);
        let carrier = entities
            .get(requested_actor)
            .expect("validated Carry owner vanished before initialization");
        if carrier.element_data().sprite.last_processed_order_id != order_id.get() {
            let carrier_position = carrier.element_data().position_map();
            let carried_direction = carrier.element_data().direction().wrapping_sub(4) & 15;
            let target = entities
                .get_mut(target_id)
                .unwrap_or_else(|| panic!("Carry target {target_id:?} vanished at initialization"));
            let element = target.element_data_mut();
            element.set_position_map(carrier_position);
            element.set_direction_instantly(carried_direction);
        }
    }

    let entity = entities
        .get_mut(requested_actor)
        .unwrap_or_else(|| panic!("ability owner {requested_actor:?} disappeared after setup"));

    // ── Listen: phase-aware animation dispatch ──
    //
    // Listen has three animation phases tracked by
    // `ActorData::listen_phase`.  The entry and exit transitions
    // are one-shot animations driven here; the middle CountingDown
    // phase is a loop driven by the idle-pose animation driver
    // plus the `listen_wait_time` countdown in
    // the selected PC owner arm.
    if kind == AbilityKind::Listen {
        let phase = listen_phase;
        let order_type = match phase {
            ListenPhase::EnterTransition => OrderType::TransitionWaitingUprightListening,
            ListenPhase::ExitTransition => OrderType::TransitionListeningWaitingUpright,
            ListenPhase::CountingDown => OrderType::Listening,
            ListenPhase::Inactive => panic!(
                "active Listen owner {entity_id:?} has Inactive phase for identity {:?}/{}/ {:?}",
                ability.sequence_id, ability.element_index, ability.order_id
            ),
        };
        // All three listen arms call `Turn()` ahead of their sprite action, so
        // the row played this tick belongs to the already-stepped direction.
        let _ = entity.position_iface_mut().turn();
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or_else(|_| {
            panic!("Listen owner {entity_id:?} has invalid animation direction")
        });
        let order_id = ability.order_id;

        let motion = if sprite_frozen {
            // Original `RHSprite::PerformAction` returns IN_PROGRESS while
            // FreezeAll is active (`RHsprite.cpp:1124-1127`), and the PC
            // Execute wrapper publishes that return through Actor::Hourglass
            // without advancing any sprite operand. The specialized Rust
            // owner reads this transient field after `tick_ability` returns,
            // so replace a stale pre-freeze DONE edge explicitly.
            entity.element_data_mut().sprite.last_motion_state =
                Some(SpriteMotionState::InProgress);
            SpriteMotionState::InProgress
        } else {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                sim,
                order_id,
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };
        if !matches!(
            motion,
            SpriteMotionState::Done | SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            return results;
        }
        let actor = entity.actor_data_mut().unwrap_or_else(|| {
            panic!("asserted Listen owner {entity_id:?} lost required actor state")
        });
        let seq_id = actor
            .active_ability
            .sequence_id
            .expect("Listen ability sequence");
        let elem_idx = actor.active_ability.element_index;
        match motion {
            SpriteMotionState::Done if actor.active_ability.done_effect_applied => {}
            SpriteMotionState::Done => {
                actor.active_ability.done_effect_applied = true;
                if phase == ListenPhase::EnterTransition {
                    // Switch to the listening pose (driven by
                    // animation.rs idle-pose fallback) and hand off
                    // to the ai.rs countdown.
                    actor.action_state = ActionState::Listening;
                    actor.listen_wait_time = crate::abilities::TIME_LISTEN_WAIT;
                    results.push(AbilityTickResult::ListenEntered {
                        actor_id: entity_id,
                    });
                } else if phase == ListenPhase::ExitTransition {
                    actor.action_state = ActionState::Waiting;
                    actor.listen_wait_time = 0;
                    results.push(AbilityTickResult::ListenDone {
                        actor_id: entity_id,
                        seq_id,
                        elem_idx,
                    });
                }
            }
            SpriteMotionState::Terminated => results.push(AbilityTickResult::Terminated {
                actor_id: entity_id,
                kind,
                seq_id,
                elem_idx,
            }),
            SpriteMotionState::Aborted => results.push(AbilityTickResult::Aborted {
                actor_id: entity_id,
                kind,
                seq_id,
                elem_idx,
                order_id: ability.order_id,
            }),
            _ => {}
        }
        return results;
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
        let phase = receive_purse_phase;
        let order_type = match phase {
            ReceivePursePhase::Receiving => OrderType::ReceivingPurse,
            ReceivePursePhase::Waiting => OrderType::WaitingWithPurse,
            ReceivePursePhase::Transition => OrderType::TransitionWaitingWithPurseWaitingUpright,
            ReceivePursePhase::Inactive => panic!(
                "active ReceivePurse owner {entity_id:?} has Inactive phase for identity {:?}/{}/ {:?}",
                ability.sequence_id, ability.element_index, ability.order_id
            ),
        };
        let direction = u16::try_from(entity.element_data().direction()).unwrap_or_else(|_| {
            panic!("ReceivePurse owner {entity_id:?} has invalid animation direction")
        });
        let order_id = ability.order_id;

        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else {
            let elem = entity.element_data_mut();
            elem.sprite.perform_action(
                sim,
                order_id,
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            )
        };
        if !matches!(
            motion,
            SpriteMotionState::Terminated | SpriteMotionState::Aborted
        ) {
            return results;
        }

        let actor = entity.actor_data_mut().unwrap_or_else(|| {
            panic!("asserted ReceivePurse owner {entity_id:?} lost required actor state")
        });
        let seq_id = actor
            .active_ability
            .sequence_id
            .expect("ReceivePurse sequence");
        let elem_idx = actor.active_ability.element_index;
        if motion == SpriteMotionState::Aborted {
            results.push(AbilityTickResult::Aborted {
                actor_id: entity_id,
                kind,
                seq_id,
                elem_idx,
                order_id: ability.order_id,
            });
            return results;
        }
        match phase {
            ReceivePursePhase::Receiving => {}
            ReceivePursePhase::Waiting => {
                results.push(AbilityTickResult::ReceivePurseRevealing {
                    beggar_id: entity_id,
                });
            }
            ReceivePursePhase::Transition => {
                actor.action_state = ActionState::Waiting;
                results.push(AbilityTickResult::ReceivePurseDone {
                    beggar_id: entity_id,
                    seq_id,
                    elem_idx,
                });
            }
            ReceivePursePhase::Inactive => unreachable!(),
        }
        results.push(AbilityTickResult::Terminated {
            actor_id: entity_id,
            kind,
            seq_id,
            elem_idx,
        });
        return results;
    }

    let order_id = ability.order_id;
    // Self-heal swaps Healing → Eating; all other abilities use
    // the canonical per-kind animation.
    let entity_id_here = entity_id;
    let order_type = if kind == AbilityKind::Heal && ability.target == Some(entity_id_here) {
        OrderType::Eating
    } else {
        ability_order_type(kind)
    };
    // These ability arms turn progressively toward the direction installed at
    // Execute-time initialization. The throws, `Hit` and `Pay` freeze the first
    // sprite frame until alignment; the rest turn for its side effect and
    // advance the action unconditionally. `Carry`, `Drop`, `Whistle` and
    // `ClimbDownFromShoulders` do not turn at all — see `docs/TURN_ARMS.md`.
    let turning = matches!(
        kind,
        AbilityKind::Hit
            | AbilityKind::Heal
            | AbilityKind::Pay
            | AbilityKind::Tie
            | AbilityKind::Eat
            | AbilityKind::ClimbOnShoulders
            | AbilityKind::ThrowApple
            | AbilityKind::ThrowStone
            | AbilityKind::ThrowPurse
            | AbilityKind::ThrowWaspNest
            | AbilityKind::ThrowNet
    ) && entity.position_iface_mut().turn();
    let frame_progression = if matches!(
        kind,
        AbilityKind::Hit
            | AbilityKind::Pay
            | AbilityKind::ThrowApple
            | AbilityKind::ThrowStone
            | AbilityKind::ThrowPurse
            | AbilityKind::ThrowWaspNest
            | AbilityKind::ThrowNet
    ) && turning
    {
        crate::sprite::FrameProgression::FrozenFirstFrame
    } else {
        crate::sprite::FrameProgression::Default
    };
    let direction = u16::try_from(entity.element_data().direction())
        .unwrap_or_else(|_| panic!("{kind:?} owner {entity_id:?} has invalid animation direction"));
    // Drive the animation through the sprite state machine.
    let motion = if sprite_frozen {
        SpriteMotionState::InProgress
    } else {
        let elem = entity.element_data_mut();
        elem.sprite.perform_action(
            sim,
            order_id,
            order_type,
            direction,
            frame_progression,
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
        if actor.wait_time != 0 {
            actor.wait_time -= 1;
        }
        actor.seek_refresh_wait = actor.wait_time;
    }

    // Only act on completion states.
    if !matches!(
        motion,
        SpriteMotionState::Done | SpriteMotionState::Terminated | SpriteMotionState::Aborted
    ) {
        return results;
    }

    let seq_id = ability.sequence_id.expect("active ability sequence");
    let elem_idx = ability.element_index;
    if motion == SpriteMotionState::Aborted {
        results.push(AbilityTickResult::Aborted {
            actor_id: entity_id,
            kind,
            seq_id,
            elem_idx,
            order_id: ability.order_id,
        });
        return results;
    }
    if motion == SpriteMotionState::Terminated {
        let actor_pos = entity.element_data().position_map();
        match kind {
            AbilityKind::Drop => {
                let actor_direction = u16::try_from(entity.element_data().direction())
                    .unwrap_or_else(|_| {
                        panic!("Drop owner {entity_id:?} has invalid terminal direction")
                    });
                let carried_posture = entity
                    .pc_data()
                    .unwrap_or_else(|| {
                        panic!("Drop owner {entity_id:?} requires PC carried-posture state")
                    })
                    .live_carried_posture();
                results.push(AbilityTickResult::DropDone {
                    carrier_id: entity_id,
                    target_id: ability.target.expect("Drop target"),
                    drop_posture: carried_posture,
                    carrier_pos: actor_pos,
                    carrier_direction: actor_direction,
                    seq_id,
                    elem_idx,
                })
            }
            AbilityKind::ClimbOnShoulders => {
                results.push(AbilityTickResult::ClimbOnShouldersDone {
                    climber_id: entity_id,
                    helper_id: ability.target.expect("climb helper"),
                    seq_id,
                    elem_idx,
                })
            }
            AbilityKind::ClimbDownFromShoulders => {
                results.push(AbilityTickResult::ClimbDownFromShouldersDone {
                    climber_id: entity_id,
                    helper_id: ability.target.expect("dismount helper"),
                    seq_id,
                    elem_idx,
                })
            }
            AbilityKind::Strangle => results.push(AbilityTickResult::StrangleDone {
                actor_id: entity_id,
                target_id: ability.target.expect("strangle target"),
                seq_id,
                elem_idx,
            }),
            _ => {}
        }
        results.push(AbilityTickResult::Terminated {
            actor_id: entity_id,
            kind,
            seq_id,
            elem_idx,
        });
        return results;
    }

    // `DONE` is an effect boundary, not ownership completion. Keep the
    // selected tuple installed until the sprite reports `TERMINATED` and
    // suppress duplicate one-shot effects on looping terminal frames.
    if motion == SpriteMotionState::Done {
        let actor = entity.actor_data_mut().unwrap();
        if actor.active_ability.done_effect_applied {
            return results;
        }
        actor.active_ability.done_effect_applied = true;
    }
    if matches!(
        kind,
        AbilityKind::Drop | AbilityKind::ClimbOnShoulders | AbilityKind::ClimbDownFromShoulders
    ) {
        return results;
    }

    // Animation finished — collect the result and clear the ability.
    let actor_pos = entity.element_data().position_map();

    // Clear ability state and reset actor.
    let actor = entity.actor_data_mut().unwrap();
    // Whistle countdown should already be 0 by the time the
    // animation completes (TIME_LISTEN_WAIT < whistle anim length),
    // but clamp defensively so a follow-up whistle can re-arm
    // cleanly in `begin_whistle`.
    if kind == AbilityKind::Whistle {
        actor.whistle_wait_time = 0;
    }

    let result = match kind {
        AbilityKind::Carry => {
            let carried_posture = entity
                .pc_data()
                .unwrap_or_else(|| {
                    panic!("Carry owner {entity_id:?} requires PC carried-posture state")
                })
                .live_carried_posture();
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
            let actor_direction = u16::try_from(entity.element_data().direction())
                .unwrap_or_else(|_| panic!("Drop owner {entity_id:?} has invalid Done direction"));
            let carried_posture = entity
                .pc_data()
                .unwrap_or_else(|| {
                    panic!("Drop owner {entity_id:?} requires PC carried-posture state")
                })
                .live_carried_posture();
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
                 in tick_ability and never reaches the generic completion match"
        ),
        AbilityKind::ThrowNet => {
            // Target position was stored in the order on the
            // owning sequence element.
            let target_pos = sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.current_order())
                .map(|o| MapPoint {
                    x: o.target_x,
                    y: o.target_y,
                })
                .unwrap_or_else(|| panic!("ThrowNet selected without its required live order"));
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
                .map(|o| MapPoint {
                    x: o.target_x,
                    y: o.target_y,
                })
                .unwrap_or_else(|| {
                    panic!("ThrowWaspNest selected without its required live order")
                });
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
                .map(|o| MapPoint {
                    x: o.target_x,
                    y: o.target_y,
                })
                .unwrap_or_else(|| panic!("ThrowPurse selected without its required live order"));
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
        AbilityKind::Strangle => AbilityTickResult::StrangleSetupDone {
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
    results
}

/// Mirror the legacy Strangle tail while its `TurnFast && TurnFast` guard
/// short-circuits before `PerformAction`.
///
/// The Original still evaluates `IsNotYetDone()` on the attacker's retained
/// sprite row and calls `PerformVirginIncrement()` on the victim.  This is
/// observable when Strangle follows an already-done walk-to-wait transition:
/// the victim's independent turning animation advances a second time in the
/// same frame even though the Strangling animation has not started yet.
fn advance_pre_action_strangle_victim_if_due(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    attacker_id: EntityId,
    victim_id: EntityId,
) {
    let due = {
        let attacker = entities
            .get(attacker_id)
            .unwrap_or_else(|| panic!("strangle attacker {attacker_id:?} vanished while turning"));
        let sprite = attacker.sprite();
        !sprite.current_scripts().is_empty()
            && sprite.current_frame >= sprite.action_done_for_row(sprite.current_row)
    };
    if due {
        entities
            .get_mut(victim_id)
            .unwrap_or_else(|| panic!("strangle victim {victim_id:?} vanished before increment"))
            .element_data_mut()
            .sprite
            .perform_virgin_increment(sim, crate::sprite::FrameProgression::Default);
    }
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
    pos: MapPoint,
    carrier_dir: i16,
    layer: u16,
    /// Carrier's current sector (copied onto the carried entity so
    /// sector-driven systems agree on which sector both occupy).
    sector: Option<crate::position_interface::SectorHandle>,
    /// Carrier's current material (derived from its obstacle).
    material: GameMaterial,
    /// The carrier's current sprite animation — determines which carry
    /// phase (lift / waiting / walking / drop) the carried entity plays.
    carrier_last_action: OrderType,
    /// Frame state to synchronize with for lift/waiting/drop phases.
    carrier_frame: u16,
    carrier_frame_count: u16,
    /// True if the carrier has `Action::LittleJohnCarry` as a contextual
    /// action — selects the LittleJohn-style carry animations (vs
    /// peasant-C style).
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
    /// The climber is still executing the animation that owns helper-side
    /// synchronization. A stale sprite row is not enough: Original performs
    /// SynchronizeAnim only from the live climbing Execute arm.
    target_live_shoulder_ability: bool,
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
pub fn sync_carried_positions(entities: &mut Entities, profiles: &crate::profiles::ProfileManager) {
    // Collect carrier snapshots first to avoid borrow conflicts.
    let mut snapshots: Vec<CarrierSnapshot> = Vec::new();
    for (pc_id, entity) in entities.pcs() {
        let carrier_id = EntityId::Pc(pc_id);
        let pc = &entity.pc;
        let Some(target_id) = pc.carried else {
            continue;
        };
        let elem = &entity.element;

        // Only the LittleJohnCarry contextual action selects the
        // LittleJohn carried-animation set. FarmerCarry grants the same
        // *ability* (carry availability checks accept either), but a
        // FarmerCarry carrier still plays the PeasantC lift/carry/drop
        // rows on the body it carries.
        let little_john_style = profiles
            .get_character(pc.profile_index)
            .map(|cp| {
                cp.contextual_actions
                    .iter()
                    .any(|&a| a == crate::profiles::Action::LittleJohnCarry)
            })
            .unwrap_or(false);

        let (last_action, frame, frame_count) = {
            let s = &elem.sprite;
            (s.last_action, s.current_frame, s.frame_count)
        };

        let carried_posture = pc.live_carried_posture();

        // Snapshot the carried entity's sprite too so the OnShoulders
        // branch can sync the helper's `TransitionHelpingClimbingUp` to
        // the climber's `ClimbingUpOnShoulders`.  For corpse-carry this is
        // unused; the carrier-driven path overwrites these fields anyway.
        let (target_last_action, target_frame, target_frame_count, target_live_shoulder_ability) =
            entities
                .get(target_id)
                .map(|e| {
                    let s = &e.element_data().sprite;
                    let live_shoulder_ability = e.actor_data().is_some_and(|actor| {
                        matches!(
                            actor.active_ability.kind,
                            Some(
                                AbilityKind::ClimbOnShoulders | AbilityKind::ClimbDownFromShoulders
                            )
                        )
                    });
                    (
                        s.last_action,
                        s.current_frame,
                        s.frame_count,
                        live_shoulder_ability,
                    )
                })
                .unwrap_or((OrderType::WaitingUpright, 0, 0, false));

        snapshots.push(CarrierSnapshot {
            carrier_id,
            target_id,
            pos: elem.position_map(),
            carrier_dir: elem.direction(),
            layer: elem.layer(),
            sector: elem.sector(),
            material: elem.material(),
            carrier_last_action: last_action,
            carrier_frame: frame,
            carrier_frame_count: frame_count,
            little_john_style,
            carried_posture,
            target_last_action,
            target_frame,
            target_frame_count,
            target_live_shoulder_ability,
        });
    }

    // Apply to each carried entity.
    for snap in snapshots {
        let on_shoulders = snap.carried_posture == Posture::OnShoulders;
        let walking_with_corpse = matches!(snap.carrier_last_action, OrderType::WalkingWithCorpse);

        let Some(target) = entities.get_mut(snap.target_id) else {
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

        // Shoulder riders follow the carrier continuously. Corpse carry only
        // rewrites the body's transform during WALKING_WITH_CORPSE; lift
        // initialization performs the one-shot initial alignment above, and
        // wait/drop/unrelated carrier actions only synchronize animation and
        // display order in the Original.
        if on_shoulders {
            let elem = target.element_data_mut();
            elem.set_position_map(snap.pos);
            elem.set_layer(snap.layer);
            elem.set_direction_instantly(carried_dir_i16);
            elem.set_sector(snap.sector);
            elem.set_material(snap.material);
            // Pin the carried's display_order just in front of the
            // carrier every frame (lift/wait/walk/drop) so the two
            // sprites stay stacked correctly when other entities cross
            // the draw list.
            let sprite = &mut elem.sprite;
            sprite.display_order_ref = Some(snap.carrier_id);
            sprite.behind_display_order_ref = false;
        } else if walking_with_corpse {
            let elem = target.element_data_mut();
            elem.set_position_map(snap.pos);
            elem.set_direction_instantly(carried_dir_i16);
            elem.sprite.display_order_ref = Some(snap.carrier_id);
            elem.sprite.behind_display_order_ref = false;
        } else {
            let sprite = &mut target.element_data_mut().sprite;
            sprite.display_order_ref = Some(snap.carrier_id);
            sprite.behind_display_order_ref = false;
        }

        // Update the carried entity's `PositionInterface` with the
        // carrier's material and the new map position, then reproject.
        //
        // The carried body keeps its OWN surface: no per-frame carry
        // animation — lift, idle-with-corpse, walk-with-corpse, or
        // walk-carrying-on-shoulders — copies the carrier's obstacle
        // onto it.  They only restamp the map position and re-project
        // through whatever plane the body already had (from where it
        // fell, or from the last explicit hand-over).  The obstacle
        // *is* copied on the one-shot hand-overs: dropping the corpse
        // and teleporting the carrier, both of which live elsewhere.
        // Copying it here instead flattened a body held above ground
        // to elevation 0 whenever the carrier itself stood on plain
        // ground.
        if on_shoulders {
            let pi = target.position_iface_mut();
            pi.set_material(snap.material);
            pi.set_map_position(snap.pos);
        }

        // ── Climb-on-shoulders branch ──────────────────────────
        // Animation sync direction is *inverted* compared to corpse-
        // carry: the climber drives `ClimbingUpOnShoulders` from
        // `tick_ability`, and the helper syncs onto it via
        // `TransitionHelpingClimbingUp`.  Once the climb finishes both
        // PCs sit on posture-driven idle poses (climber →
        // WaitingOnShoulders, helper → WaitingCarryingOnShoulders);
        // during the helper's `WalkingCarryingOnShoulders` we force
        // `WaitingOnShoulders` on the climber.
        if on_shoulders {
            let carried_anim = match snap.carrier_last_action {
                // Helper walking with PC on shoulders — climber rides idle.
                OrderType::WalkingCarryingOnShoulders => Some(OrderType::WaitingOnShoulders),
                // A live climb is driven by tick_ability. Once both PCs are
                // idle, the rider's own WaitingOnShoulders Execute owns its
                // independent PerformAction timer; Original does not sync it
                // to WaitingCarryingOnShoulders (RHelementactorpc.cpp:
                // 4648-4687).
                _ => None,
            };
            if let Some(anim) = carried_anim {
                let sprite = &mut target.element_data_mut().sprite;
                let is_walking = matches!(
                    snap.carrier_last_action,
                    OrderType::WalkingCarryingOnShoulders
                );
                if is_walking {
                    // TODO(original-parity): WalkingCarryingOnShoulders does
                    // ForceAnimation + ResetSpriteFrame only on initialization,
                    // then calls the rider's PerformAction every frame
                    // (RHelementactorpc.cpp:3724-3751). This per-frame force
                    // remains a separate walking-ownership gap.
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
            let helper_anim = snap
                .target_live_shoulder_ability
                .then(|| match snap.target_last_action {
                    OrderType::ClimbingUpOnShoulders => {
                        Some(OrderType::TransitionHelpingClimbingUp)
                    }
                    OrderType::ClimbingDownFromShoulders => {
                        Some(OrderType::TransitionHelpingClimbingDown)
                    }
                    _ => None,
                })
                .flatten();
            if let Some(anim) = helper_anim
                && let Some(helper) = entities.get_mut(snap.carrier_id)
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
            // Waiting with corpse — synced with carrier's sprite.
            OrderType::WaitingWithCorpse => {
                if snap.little_john_style {
                    Some(OrderType::BeingCarriedLittleJohn)
                } else {
                    Some(OrderType::BeingCarriedPeasantC)
                }
            }
            // `pc.carried` is latched while TakeCorpse is translated, before
            // the transition order reaches Execute. Original does not set
            // `mpCarried` or publish a carried-body animation until that
            // transition initializes (RHelementactorpc.cpp:4842-4909).
            // Unrelated/pre-transition carrier actions therefore leave the
            // body's existing animation untouched.
            _ => None,
        };

        if let Some(anim) = carried_anim {
            // For WalkingWithCorpse use `force_animation` which resets
            // frame/frame_count to 0.  Everything else forces the
            // sprite row and then syncs the frame with the carrier.
            if walking_with_corpse {
                let sprite = &mut target.element_data_mut().sprite;
                sprite.force_animation(anim, carried_dir_u16);
            } else {
                let existing_direction = u16::try_from(target.element_data().direction())
                    .unwrap_or_else(|_| panic!("carried corpse has negative direction"));
                let sprite = &mut target.element_data_mut().sprite;
                sprite.force_sprite_row(anim, existing_direction);
                sprite.synchronize_anim(snap.carrier_frame, snap.carrier_frame_count);
            }
        }
    }
}

/// Synchronize the corpse carried by one PC from inside that PC's
/// `WalkingWithCorpse` Execute arm.
///
/// Original performs this immediately after `PerformMotion`, before the
/// carrier returns from its actor slot (`RHelementactorpc.cpp:5000-5032`).
/// The broad end-of-frame carry pass remains authoritative for the other
/// carry phases whose exact owner boundary has not been established.
pub(crate) fn sync_walking_corpse_for_carrier(
    entities: &mut Entities,
    profiles: &crate::profiles::ProfileManager,
    carrier_id: EntityId,
) {
    let Some(carrier) = entities.get(carrier_id) else {
        panic!("WalkingWithCorpse carrier {carrier_id:?} disappeared during Execute")
    };
    let Some(pc) = carrier.pc_data() else {
        panic!("WalkingWithCorpse owner {carrier_id:?} is not a PC")
    };
    let Some(target_id) = pc.carried else {
        panic!("WalkingWithCorpse carrier {carrier_id:?} has no carried actor")
    };
    let position = carrier.element_data().position_map();
    let carrier_direction = carrier.element_data().direction();
    let little_john_style = profiles
        .get_character(pc.profile_index)
        .map(|profile| {
            profile
                .contextual_actions
                .contains(&crate::profiles::Action::LittleJohnCarry)
        })
        .unwrap_or(false);

    let target = entities.get_mut(target_id).unwrap_or_else(|| {
        panic!("WalkingWithCorpse carrier {carrier_id:?} references missing actor {target_id:?}")
    });
    let carried_direction = carrier_direction.wrapping_sub(4) & 15;
    let carried_direction_u16 = u16::try_from(carried_direction)
        .unwrap_or_else(|_| panic!("carried corpse has negative direction"));
    let element = target.element_data_mut();
    element.set_position_map(position);
    element.set_direction_instantly(carried_direction);
    element.sprite.display_order_ref = Some(carrier_id);
    element.sprite.behind_display_order_ref = false;
    element.sprite.force_animation(
        if little_john_style {
            OrderType::BeingCarriedLittleJohn
        } else {
            OrderType::BeingCarriedPeasantC
        },
        carried_direction_u16,
    );
}

/// Synchronize the carried body's final drop frame from the carrier before
/// `DropCorpse` severs their link.
///
/// The ordinary end-of-frame carry pass cannot provide this edge: Original's
/// PC Execute arm calls `SynchronizeAnim` and then `DropCorpse` in one stack,
/// while Rust drains the terminal side effect before that later global pass.
pub(crate) fn sync_terminal_corpse_drop_animation(
    entities: &mut Entities,
    profiles: &crate::profiles::ProfileManager,
    carrier_id: EntityId,
) {
    let (target_id, profile_index, carrier_frame, carrier_frame_count) = {
        let carrier = entities
            .get(carrier_id)
            .unwrap_or_else(|| panic!("terminal corpse-drop carrier {carrier_id:?} disappeared"));
        let pc = carrier
            .pc_data()
            .unwrap_or_else(|| panic!("terminal corpse-drop carrier {carrier_id:?} is not a PC"));
        let target_id = pc.carried.unwrap_or_else(|| {
            panic!("terminal corpse-drop carrier {carrier_id:?} has no carried body")
        });
        let sprite = &carrier.element_data().sprite;
        (
            target_id,
            pc.profile_index,
            sprite.current_frame,
            sprite.frame_count,
        )
    };
    let little_john_style = profiles
        .get_character(profile_index)
        .map(|profile| {
            profile
                .contextual_actions
                .iter()
                .any(|&action| action == crate::profiles::Action::LittleJohnCarry)
        })
        .unwrap_or(false);
    let animation = if little_john_style {
        OrderType::BeingDroppedLittleJohn
    } else {
        OrderType::BeingDroppedPeasantC
    };
    let target = entities.get_mut(target_id).unwrap_or_else(|| {
        panic!("terminal corpse-drop carrier {carrier_id:?} lost body {target_id:?}")
    });
    // RHElement::SynchronizeAnim(anim, other) forces `anim` with the carried
    // element's current direction. DropCorpse rotates the body relative to the
    // carrier only afterward (RHelementactorpc.cpp:4946-4962, 6475).
    let carried_direction = u16::try_from(target.element_data().direction()).unwrap_or_else(|_| {
        panic!("terminal corpse-drop body {target_id:?} has negative direction")
    });
    let sprite = &mut target.element_data_mut().sprite;
    sprite.force_sprite_row(animation, carried_direction);
    sprite.synchronize_anim(carrier_frame, carrier_frame_count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorCivilian, ActorPc, CivilianData, ElementData, ElementKind, HumanData, NpcData, PcData,
    };
    use crate::sequence::SequenceElement;
    use crate::sight_obstacle::ObstacleList;

    fn launch_ability_element(
        manager: &mut SequenceManager,
        command: crate::element::Command,
        owner: EntityId,
    ) -> SequenceId {
        let seq_id = manager.launch_element(SequenceElement::new(1, command, Some(owner)));
        manager.element_in_progress(seq_id, 0);
        seq_id
    }

    fn take_corpse_translation_fixture() -> (Entities, EntityId, EntityId, EntityId) {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::CarryingCorpse,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..Default::default()
            },
        })));
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Tied,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData {
                unconscious: true,
                ..Default::default()
            },
            pc: PcData {
                life_points: 1,
                ..Default::default()
            },
        })));
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Lying,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData {
                unconscious: true,
                ..Default::default()
            },
            pc: PcData {
                life_points: 1,
                ..Default::default()
            },
        })));
        (
            entities,
            EntityId::Pc(crate::entity_id::PcId(0)),
            EntityId::Pc(crate::entity_id::PcId(1)),
            EntityId::Pc(crate::entity_id::PcId(2)),
        )
    }

    /// Seed3 linux3 Savegame_007 replay-011 and Savegame_008 replay-028:
    /// restored PC 194 is CarryingCorpse/Waiting and remains reciprocally
    /// linked to one body while TakeCorpse targets a second available body.
    /// Original ignores `mpCarried` during validity, installs action 188, and
    /// replaces that pointer only at the first Execute boundary.
    #[test]
    fn take_corpse_translation_accepts_restored_carry_link_and_first_execute_replaces_it() {
        let (mut entities, carrier, old_target, target) = take_corpse_translation_fixture();
        entities
            .get_mut(carrier)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(old_target);
        entities
            .get_mut(carrier)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
        entities
            .get_mut(old_target)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(carrier);

        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::TakeCorpse, carrier);
        let mut next_order_id = 300;
        assert_eq!(
            begin_carry(
                &mut entities,
                &mut manager,
                carrier,
                target,
                seq_id,
                0,
                &mut next_order_id,
            ),
            BeginResult::Started
        );

        let element = manager.get_element(seq_id, 0).unwrap();
        assert_eq!(element.command, crate::element::Command::TakeCorpse);
        assert_eq!(
            element.current_order().unwrap().order_type,
            OrderType::TransitionWaitingUprightCarryingCorpse,
            "manager-phase translation must expose Original action 188 instead of falling back to Wait"
        );
        assert_eq!(
            entities.get(carrier).unwrap().pc_data().unwrap().carried,
            Some(old_target),
            "translation must not mutate the restored relationship before first Execute"
        );
        assert_eq!(
            entities
                .get(carrier)
                .unwrap()
                .pc_data()
                .unwrap()
                .live_carried_posture(),
            Posture::Tied,
            "translation must preserve the old body's restored drop posture through its prefix"
        );
        assert_eq!(
            entities
                .get(old_target)
                .unwrap()
                .human_data()
                .unwrap()
                .carrier,
            Some(carrier)
        );
        assert_eq!(
            entities.get(target).unwrap().human_data().unwrap().carrier,
            None
        );

        initialize_carry_relationship(&mut entities, carrier, target);
        assert_eq!(
            entities.get(carrier).unwrap().pc_data().unwrap().carried,
            Some(target),
            "first Execute must replace the restored mpCarried link"
        );
        assert_eq!(
            entities
                .get(carrier)
                .unwrap()
                .pc_data()
                .unwrap()
                .live_carried_posture(),
            Posture::Lying,
            "first Execute must snapshot the new target's posture"
        );
        assert_eq!(
            entities.get(target).unwrap().human_data().unwrap().carrier,
            Some(carrier)
        );
        entities
            .get_mut(target)
            .unwrap()
            .set_posture(Posture::Carried);
        initialize_carry_relationship(&mut entities, carrier, target);
        assert_eq!(
            entities
                .get(carrier)
                .unwrap()
                .pc_data()
                .unwrap()
                .live_carried_posture(),
            Posture::Lying,
            "later Execute calls must not replace the first-Execute posture with Carried"
        );
        assert_eq!(
            entities
                .get(old_target)
                .unwrap()
                .human_data()
                .unwrap()
                .carrier,
            Some(carrier),
            "Original does not clear the old body's reciprocal pointer here"
        );
    }

    #[test]
    fn take_corpse_translation_rejects_inactive_or_foreign_carried_target() {
        for invalid in ["inactive", "foreign_carrier"] {
            let (mut entities, carrier, _old_target, target) = take_corpse_translation_fixture();
            match invalid {
                "inactive" => entities.get_mut(target).unwrap().element_data_mut().active = false,
                "foreign_carrier" => {
                    entities
                        .get_mut(target)
                        .unwrap()
                        .human_data_mut()
                        .unwrap()
                        .carrier = Some(EntityId::Pc(crate::entity_id::PcId(99)));
                }
                _ => unreachable!(),
            }
            let mut manager = SequenceManager::new();
            let seq_id =
                launch_ability_element(&mut manager, crate::element::Command::TakeCorpse, carrier);
            let mut next_order_id = 300;

            assert_eq!(
                begin_carry(
                    &mut entities,
                    &mut manager,
                    carrier,
                    target,
                    seq_id,
                    0,
                    &mut next_order_id,
                ),
                BeginResult::Impossible,
                "Original rejects the {invalid} TakeCorpse target"
            );
            assert!(manager.get_element(seq_id, 0).unwrap().orders.is_empty());
        }
    }

    fn corpse_carry_fixture(carrier_action: OrderType) -> (Entities, EntityId, EntityId) {
        let mut entities = Entities::new();
        let mut carrier = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        carrier.element.set_position_map(MapPoint::new(80.0, 90.0));
        carrier.element.set_direction_instantly(9);
        carrier.element.set_layer(9);
        carrier.element.set_sector(Some(
            crate::position_interface::SectorHandle::new(8).unwrap(),
        ));
        carrier
            .element
            .set_material(crate::element::GameMaterial::Stone);
        carrier.element.sprite.last_action = carrier_action;
        let mut body = ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                posture: Posture::Carried,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData::default(),
        };
        body.element.set_position_map(MapPoint::new(10.0, 20.0));
        body.element.set_direction_instantly(4);
        body.element.set_layer(3);
        body.element.set_sector(Some(
            crate::position_interface::SectorHandle::new(7).unwrap(),
        ));
        body.element
            .set_material(crate::element::GameMaterial::Wood);
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[OrderType::BeingCarriedPeasantC as usize] = 100;
        body.element.sprite.conversion = std::sync::Arc::new(conversion);
        let mut scripts = vec![crate::sprite_script::SpriteScript::default(); 116];
        scripts[104].frame_ids = vec![0, 1, 2, 3];
        body.element.sprite.scripts = std::sync::Arc::new(scripts);
        body.element.sprite.force_sprite_row_raw(104);
        body.element.sprite.last_action = OrderType::BeingCarriedPeasantC;
        entities.push(Some(Entity::Pc(carrier)));
        entities.push(Some(Entity::Civilian(body)));
        let carrier_id = entities.id_at_legacy_slot(0).unwrap();
        let body_id = entities.id_at_legacy_slot(1).unwrap();
        entities
            .get_mut(carrier_id)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(body_id);
        entities
            .get_mut(body_id)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(carrier_id);
        (entities, carrier_id, body_id)
    }

    #[test]
    fn unrelated_carrier_action_preserves_corpse_transform_and_facing_row() {
        let (mut entities, _carrier, body) =
            corpse_carry_fixture(OrderType::TransitionWaitingUprightHelpingClimbing);
        {
            let sprite = &mut entities.get_mut(body).unwrap().element_data_mut().sprite;
            sprite.force_sprite_row_raw(777);
            sprite.last_action = OrderType::BeingTied;
            sprite.current_frame = 3;
            sprite.frame_count = 9;
        }
        let before = entities.get(body).unwrap().element_data();
        let position = before.position_map();
        let direction = before.direction();
        let sprite = (
            before.sprite.current_row,
            before.sprite.last_action,
            before.sprite.current_frame,
            before.sprite.frame_count,
        );

        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());

        let body = entities.get(body).unwrap();
        assert_eq!(body.element_data().position_map(), position);
        assert_eq!(body.element_data().direction(), direction);
        assert_eq!(
            (
                body.element_data().sprite.current_row,
                body.element_data().sprite.last_action,
                body.element_data().sprite.current_frame,
                body.element_data().sprite.frame_count,
            ),
            sprite,
            "a latched carry link must not publish a carried visual before the lift executes"
        );
    }

    #[test]
    fn waiting_with_corpse_publishes_idle_carried_visual() {
        let (mut entities, carrier, body) = corpse_carry_fixture(OrderType::WaitingWithCorpse);
        {
            let carrier_sprite = &mut entities.get_mut(carrier).unwrap().element_data_mut().sprite;
            carrier_sprite.current_frame = 2;
            carrier_sprite.frame_count = 7;
        }

        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());

        let sprite = &entities.get(body).unwrap().element_data().sprite;
        assert_eq!(sprite.last_action, OrderType::BeingCarriedPeasantC);
        assert_eq!((sprite.current_frame, sprite.frame_count), (2, 7));
    }

    #[test]
    fn lift_transition_publishes_being_lifted_visual() {
        let (mut entities, carrier, body) =
            corpse_carry_fixture(OrderType::TransitionWaitingUprightCarryingCorpse);
        {
            let carrier_sprite = &mut entities.get_mut(carrier).unwrap().element_data_mut().sprite;
            carrier_sprite.current_frame = 1;
            carrier_sprite.frame_count = 6;
        }
        {
            let sprite = &mut entities.get_mut(body).unwrap().element_data_mut().sprite;
            let mut conversion = (*sprite.conversion).clone();
            conversion[OrderType::BeingLiftedPeasantC as usize] = 200;
            sprite.conversion = std::sync::Arc::new(conversion);
            let mut scripts = vec![crate::sprite_script::SpriteScript::default(); 216];
            scripts[204].frame_ids = vec![0, 1, 2, 3];
            sprite.scripts = std::sync::Arc::new(scripts);
        }

        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());

        let sprite = &entities.get(body).unwrap().element_data().sprite;
        assert_eq!(sprite.last_action, OrderType::BeingLiftedPeasantC);
        assert_eq!((sprite.current_frame, sprite.frame_count), (1, 6));
    }

    #[test]
    fn walking_with_corpse_updates_transform_and_carrier_relative_direction() {
        let (mut entities, carrier, body) = corpse_carry_fixture(OrderType::WalkingWithCorpse);
        let carrier_position = entities.get(carrier).unwrap().element_data().position_map();
        let before = entities.get(body).unwrap().element_data();
        let layer = before.layer();
        let sector = before.sector();
        let material = before.material();

        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());

        let body = entities.get(body).unwrap();
        assert_eq!(body.element_data().position_map(), carrier_position);
        assert_eq!(body.element_data().direction(), 5);
        assert_eq!(body.element_data().layer(), layer);
        assert_eq!(body.element_data().sector(), sector);
        assert_eq!(body.element_data().material(), material);
        assert_eq!(
            body.element_data().sprite.last_action,
            OrderType::BeingCarriedPeasantC
        );
    }

    #[test]
    fn terminal_corpse_drop_sync_uses_body_facing_before_drop_rotation() {
        let (mut entities, carrier, body) =
            corpse_carry_fixture(OrderType::TransitionCarryingCorpseWaitingUpright);
        {
            let sprite = &mut entities.get_mut(body).unwrap().element_data_mut().sprite;
            let mut conversion = (*sprite.conversion).clone();
            conversion[OrderType::BeingDroppedPeasantC as usize] = 200;
            sprite.conversion = std::sync::Arc::new(conversion);
            sprite.scripts =
                std::sync::Arc::new(vec![crate::sprite_script::SpriteScript::default(); 216]);
        }

        // The body still faces 4 while the carrier faces 9. Original selects
        // the drop row with 4 here; DropCorpse changes the body to 5 later.
        sync_terminal_corpse_drop_animation(
            &mut entities,
            &crate::profiles::ProfileManager::default(),
            carrier,
        );

        let body = entities.get(body).unwrap();
        assert_eq!(body.element_data().direction(), 4);
        assert_eq!(body.element_data().sprite.current_row, 204);
        assert_eq!(
            body.element_data().sprite.last_action,
            OrderType::BeingDroppedPeasantC
        );
    }

    #[test]
    fn pay_translation_preserves_direction_goal_until_execute_initialization() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData {
                beggar_scroll_sets: Some(vec![vec![]]),
                ..Default::default()
            },
        })));
        let pc = entities.id_at_legacy_slot(0).unwrap();
        let beggar = entities.id_at_legacy_slot(1).unwrap();
        entities
            .get_mut(pc)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(1);
        entities
            .get_mut(beggar)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(6);

        let mut manager = SequenceManager::new();
        let seq_id = manager.launch_element(SequenceElement::new_interaction(
            1,
            Command::Pay,
            Some(pc),
            Some(beggar),
        ));
        let mut next_id = 1;
        assert_eq!(
            begin_pay(
                &mut entities,
                &mut manager,
                pc,
                beggar,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );

        assert_eq!(
            entities
                .get(pc)
                .unwrap()
                .position_iface()
                .get_direction_goal()
                .as_u8(),
            1,
            "RHCOMMAND_PAY translation only installs its Paying order"
        );
        assert_eq!(
            manager
                .get_element(seq_id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::Paying
        );
    }

    #[test]
    fn loaded_tying_order_reconstructs_rust_only_ability_latch() {
        let mut entities = Entities::new();
        for _ in 0..2 {
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            })));
        }
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut manager, Command::TieCmd, owner);
        let order_id = std::num::NonZeroU32::new(41).unwrap();
        let mut order = Order::new(OrderType::Tying, 12.0, 34.0, order_id);
        order.antagonist = Some(target);
        order.target_actor = Some(target.index());
        manager.push_order_on(seq_id, 0, order);

        restore_loaded_active_abilities(&mut entities, &manager);

        let restored = &entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability;
        assert_eq!(restored.kind, Some(AbilityKind::Tie));
        assert_eq!(restored.sequence_id, Some(seq_id));
        assert_eq!(restored.element_index, 0);
        assert_eq!(restored.target, Some(target));
        assert_eq!(restored.order_id, Some(order_id));
        assert!(!restored.done_effect_applied);
    }

    #[test]
    fn loaded_ability_behind_transition_prefix_reconstructs_from_first_command_order() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut manager, Command::EatCmd, owner);
        let transition_id = std::num::NonZeroU32::new(40).unwrap();
        let eating_id = std::num::NonZeroU32::new(41).unwrap();
        manager.push_order_on(
            seq_id,
            0,
            Order::new(
                OrderType::TransitionWalkingUprightWaitingUpright,
                0.0,
                0.0,
                transition_id,
            ),
        );
        manager.push_order_on(
            seq_id,
            0,
            Order::new(OrderType::Eating, 0.0, 0.0, eating_id),
        );
        manager
            .get_element_mut(seq_id, 0)
            .unwrap()
            .num_transition_orders = 1;

        restore_loaded_active_abilities(&mut entities, &manager);

        let restored = &entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability;
        assert_eq!(restored.kind, Some(AbilityKind::Eat));
        assert_eq!(restored.sequence_id, Some(seq_id));
        assert_eq!(restored.element_index, 0);
        assert_eq!(restored.order_id, Some(eating_id));
        assert!(!restored.done_effect_applied);
    }

    #[test]
    fn hit_translation_preserves_live_movement_state_and_facing() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let attacker = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        let retained_goal = MapPoint::new(70.0, 80.0);
        {
            let entity = entities.get_mut(attacker).unwrap();
            entity.element_data_mut().set_direction_instantly(8);
            entity.position_iface_mut().set_map_goal(retained_goal);
            entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        }
        entities
            .get_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 0.0));

        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::HitCmd, attacker);
        let mut next_id = 1;
        assert_eq!(
            begin_hit(
                &mut entities,
                &mut manager,
                attacker,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );

        let attacker = entities.get(attacker).unwrap();
        assert_eq!(
            attacker.actor_data().unwrap().action_state,
            ActionState::Moving
        );
        assert_eq!(attacker.element_data().direction(), 8);
        assert_eq!(attacker.position_iface().map_goal(), retained_goal);
        let order = manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap();
        assert_eq!(order.order_type, OrderType::Hitting);
        assert!(!order.compute_direction);
    }

    #[test]
    fn strangle_translation_preserves_live_movement_state_for_stale_dead_target() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai_brain: crate::element::AiBrain::Friendly(Box::default()),
                ..Default::default()
            },
            civilian: CivilianData::default(),
        })));
        let attacker = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        {
            let target = entities.get_mut(target).unwrap();
            target.element_data_mut().posture = Posture::Dead;
            target.npc_data_mut().unwrap().life_points = 0;
        }
        entities
            .get_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Moving;

        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut manager, Command::StrangleCmd, attacker);
        let mut next_id = 1;
        assert_eq!(
            begin_strangle(
                &mut entities,
                &mut manager,
                attacker,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );

        assert_eq!(
            entities
                .get(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            ActionState::Moving,
            "Strangle translation must not overwrite the post-seek movement state"
        );
        let order = manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap();
        assert_eq!(order.order_type, OrderType::Strangling);
        assert!(!order.compute_direction);
    }

    #[test]
    fn heal_selection_defers_facing_until_first_execute() {
        let mut entities = Entities::new();
        for _ in 0..2 {
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            })));
        }
        let healer = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        entities
            .get_mut(healer)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(11);
        {
            let target = entities.get_mut(target).unwrap();
            target.pc_data_mut().unwrap().life_points = 50;
            target
                .element_data_mut()
                .set_position_map(MapPoint::new(-20.0, 10.0));
        }

        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut manager, Command::HealCmd, healer);
        let mut next_id = 1;
        assert_eq!(
            begin_heal(
                &mut entities,
                &mut manager,
                healer,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );

        assert_eq!(
            entities.get(healer).unwrap().element_data().direction(),
            11,
            "selecting Heal must not run RHANIMATION_HEALING initialization"
        );
        let order = manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap();
        assert_eq!(order.order_type, OrderType::Healing);
        assert_eq!(order.target_actor, Some(target.index()));
        assert!(!order.compute_direction);
    }

    #[test]
    fn heal_translation_does_not_prevalidate_full_health_target() {
        let mut entities = Entities::new();
        for _ in 0..2 {
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            })));
        }
        let healer = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        entities
            .get_mut(target)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .life_points = LIFEPOINTS_PC;

        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut manager, Command::HealCmd, healer);
        let mut next_id = 1;
        assert_eq!(
            begin_heal(
                &mut entities,
                &mut manager,
                healer,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started,
            "Translate must install Healing before Execute checks whether the victim is still injured"
        );
        assert_eq!(
            manager
                .get_element(seq_id, 0)
                .and_then(SequenceElement::current_order)
                .map(|order| order.order_type),
            Some(OrderType::Healing)
        );
    }

    #[test]
    fn hit_turns_before_advancing_its_first_animation_frame() {
        let mut entities = Entities::new();
        for _ in 0..2 {
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: HumanData::default(),
                pc: PcData::default(),
            })));
        }
        let attacker = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        {
            let entity = entities.get_mut(attacker).unwrap();
            entity.element_data_mut().set_direction_instantly(1);
            entity.element_data_mut().set_direction_goal(4);
            entity.position_iface_mut().deviated = false;
        }

        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::HitCmd, attacker);
        let mut next_id = 1;
        assert_eq!(
            begin_hit(
                &mut entities,
                &mut manager,
                attacker,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );
        let sim = crate::sim_rng::test_context();

        for expected_direction in [2, 3, 4] {
            assert!(tick_ability(&sim, &mut entities, &manager, attacker, false).is_empty());
            let entity = entities.get(attacker).unwrap();
            assert_eq!(entity.element_data().direction(), expected_direction);
            assert_eq!(
                entity.element_data().sprite.current_frame,
                0,
                "Hitting must remain on its first frame while Turn reports progress"
            );
        }
    }

    #[test]
    fn strangle_turn_fast_short_circuits_attacker_before_victim() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData::default(),
        })));
        let attacker = entities.id_at_legacy_slot(0).unwrap();
        let victim = entities.id_at_legacy_slot(1).unwrap();
        entities
            .get_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_goal(4);
        entities
            .get_mut(victim)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(8);
        entities
            .get_mut(victim)
            .unwrap()
            .element_data_mut()
            .set_direction_goal(4);
        entities
            .get_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_ability = ActiveAbility {
            kind: Some(AbilityKind::Strangle),
            sequence_id: Some(SequenceId(7)),
            element_index: 3,
            target: Some(victim),
            order_id: std::num::NonZeroU32::new(11),
            done_effect_applied: false,
            strangle_initialized: true,
        };
        let manager = SequenceManager::new();
        let sim = crate::sim_rng::test_context();

        for expected_attacker in [2, 4] {
            assert!(tick_ability(&sim, &mut entities, &manager, attacker, false).is_empty());
            assert_eq!(
                entities.get(attacker).unwrap().element_data().direction(),
                expected_attacker
            );
            assert_eq!(entities.get(victim).unwrap().element_data().direction(), 8);
        }
        assert!(tick_ability(&sim, &mut entities, &manager, attacker, false).is_empty());
        assert_eq!(
            entities.get(attacker).unwrap().element_data().direction(),
            4
        );
        assert_eq!(entities.get(victim).unwrap().element_data().direction(), 6);
        assert_eq!(
            entities
                .get(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_ability
                .sequence_id,
            Some(SequenceId(7)),
            "turning must retain the exact owner/sequence/element/order identity",
        );
    }

    #[test]
    fn listen_uses_three_real_sequence_orders_with_stable_identity() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::EnterListen, owner);
        let mut next_id = 100;
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            actions: [
                crate::profiles::Action::Listen,
                crate::profiles::Action::NoAction,
                crate::profiles::Action::NoAction,
            ],
            ..Default::default()
        });

        assert_eq!(
            begin_listen(
                &mut entities,
                &profiles,
                &mut manager,
                owner,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );
        let element = manager.get_element_mut(seq_id, 0).unwrap();
        let mut actual = Vec::new();
        while let Some(order) = element.pop_current_order() {
            actual.push((order.order_type, order.order_id));
        }
        assert_eq!(
            actual.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![
                OrderType::TransitionWaitingUprightListening,
                OrderType::Listening,
                OrderType::TransitionListeningWaitingUpright,
            ]
        );
        assert_eq!(
            actual.iter().map(|entry| entry.1.get()).collect::<Vec<_>>(),
            vec![100, 101, 102]
        );
        assert_eq!(next_id, 103);
        assert_eq!(
            entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_ability
                .order_id,
            Some(actual[0].1)
        );
    }

    #[test]
    fn frozen_listen_entry_publishes_in_progress_without_advancing_sprite() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::EnterListen, owner);
        let mut next_id = 200;
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            actions: [
                crate::profiles::Action::Listen,
                crate::profiles::Action::NoAction,
                crate::profiles::Action::NoAction,
            ],
            ..Default::default()
        });
        assert_eq!(
            begin_listen(
                &mut entities,
                &profiles,
                &mut manager,
                owner,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );

        let sprite = &mut entities.get_mut(owner).unwrap().element_data_mut().sprite;
        sprite.current_row = 1876;
        sprite.current_frame = 2;
        sprite.frame_count = 2;
        sprite.action_done_frame = 2;
        sprite.action_done_counter = 2;
        sprite.last_motion_state = Some(SpriteMotionState::Done);
        let operands_before = (
            sprite.current_row,
            sprite.current_frame,
            sprite.frame_count,
            sprite.action_done_frame,
            sprite.action_done_counter,
        );

        assert!(
            tick_ability(
                &crate::sim_rng::test_context(),
                &mut entities,
                &manager,
                owner,
                true,
            )
            .is_empty()
        );

        let sprite = &entities.get(owner).unwrap().element_data().sprite;
        assert_eq!(
            (
                sprite.current_row,
                sprite.current_frame,
                sprite.frame_count,
                sprite.action_done_frame,
                sprite.action_done_counter,
            ),
            operands_before,
            "FreezeAll must not advance the action-point sprite operands"
        );
        assert_eq!(
            sprite.last_motion_state,
            Some(SpriteMotionState::InProgress),
            "the Listen owner envelope must not retain the pre-freeze DONE edge"
        );
    }

    #[test]
    fn carry_creates_only_its_canonical_sequence_order() {
        let mut entities = Entities::new();
        let mut carrier_entity = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..Default::default()
            },
        };
        carrier_entity
            .element
            .set_position_map(MapPoint::new(80.0, 90.0));
        carrier_entity.element.set_direction_instantly(9);
        let mut carrier_conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        carrier_conversion[OrderType::TransitionWaitingUprightCarryingCorpse as usize] = 0;
        carrier_entity.element.sprite.conversion = std::sync::Arc::new(carrier_conversion);
        let mut carrier_scripts = vec![crate::sprite_script::SpriteScript::default(); 16];
        carrier_scripts[0].frame_ids = vec![0];
        carrier_scripts[0].delays = vec![1];
        carrier_scripts[0].distances = vec![0];
        carrier_scripts[0].offsets = vec![crate::coordinates::SpriteFrameOffset::ZERO];
        carrier_scripts[0].sound_ids = vec![0];
        carrier_entity.element.sprite.scripts = std::sync::Arc::new(carrier_scripts);
        let mut target_entity = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Dead,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 0,
                ..Default::default()
            },
        };
        target_entity
            .element
            .set_position_map(MapPoint::new(10.0, 20.0));
        target_entity.element.set_direction_instantly(4);
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[OrderType::BeingLiftedPeasantC as usize] = 100;
        target_entity.element.sprite.conversion = std::sync::Arc::new(conversion);
        target_entity.element.sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript::default(); 116]);
        entities.push(Some(Entity::Pc(carrier_entity)));
        entities.push(Some(Entity::Pc(target_entity)));
        let carrier = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::TakeCorpse, carrier);
        let mut next_id = 300;

        assert_eq!(
            begin_carry(
                &mut entities,
                &mut manager,
                carrier,
                target,
                seq_id,
                0,
                &mut next_id,
            ),
            BeginResult::Started
        );
        assert_eq!(next_id, 301);
        assert_eq!(
            entities.get(carrier).unwrap().pc_data().unwrap().carried,
            None,
            "translation before the pickup order's first Execute must not publish mpCarried"
        );
        let target_entity = entities.get(target).unwrap();
        assert_eq!(
            target_entity.human_data().unwrap().carrier,
            None,
            "translation before the pickup order's first Execute must not publish SetCarrier"
        );
        assert_eq!(
            target_entity.element_data().position_map(),
            MapPoint::new(10.0, 20.0)
        );
        assert_eq!(target_entity.element_data().direction(), 4);

        let _ = tick_ability(
            &crate::sim_rng::SimulationContext::with_seed(1),
            &mut entities,
            &manager,
            carrier,
            false,
        );

        let target_entity = entities.get(target).unwrap();
        assert_eq!(
            entities.get(carrier).unwrap().pc_data().unwrap().carried,
            Some(target)
        );
        assert_eq!(target_entity.human_data().unwrap().carrier, Some(carrier));
        assert_eq!(
            target_entity.element_data().position_map(),
            MapPoint::new(80.0, 90.0)
        );
        assert_eq!(target_entity.element_data().direction(), 5);

        let element = manager.get_element_mut(seq_id, 0).unwrap();
        let order = element.pop_current_order().expect("canonical Carry order");
        assert_eq!(
            (order.order_type, order.order_id.get()),
            (OrderType::TransitionWaitingUprightCarryingCorpse, 300)
        );
        assert!(element.pop_current_order().is_none());

        let carrier_entity = entities.get_mut(carrier).unwrap();
        carrier_entity
            .element_data_mut()
            .set_position_map(MapPoint::new(120.0, 140.0));
        carrier_entity
            .element_data_mut()
            .set_direction_instantly(12);
        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());
        let target_entity = entities.get(target).unwrap();
        assert_eq!(
            target_entity.element_data().position_map(),
            MapPoint::new(80.0, 90.0),
            "the live lift animation must not continuously restamp the corpse"
        );
        assert_eq!(target_entity.element_data().direction(), 5);
    }

    #[test]
    fn carry_initialization_preserves_action_state_until_the_lift_finishes() {
        for initial_action_state in [ActionState::Moving, ActionState::Waiting] {
            let mut entities = Entities::new();
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    posture: Posture::Upright,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    action_state: initial_action_state,
                    ..Default::default()
                },
                human: HumanData::default(),
                pc: PcData {
                    life_points: 100,
                    ..Default::default()
                },
            })));
            entities.push(Some(Entity::Pc(ActorPc {
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    posture: Posture::Dead,
                    ..Default::default()
                },
                actor: Default::default(),
                human: HumanData::default(),
                pc: PcData {
                    life_points: 0,
                    ..Default::default()
                },
            })));
            let carrier = entities.id_at_legacy_slot(0).unwrap();
            let target = entities.id_at_legacy_slot(1).unwrap();
            let mut manager = SequenceManager::new();
            let seq_id =
                launch_ability_element(&mut manager, crate::element::Command::TakeCorpse, carrier);
            let mut next_id = 300;

            assert_eq!(
                begin_carry(
                    &mut entities,
                    &mut manager,
                    carrier,
                    target,
                    seq_id,
                    0,
                    &mut next_id,
                ),
                BeginResult::Started
            );
            assert_eq!(
                entities
                    .get(carrier)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .action_state,
                initial_action_state,
                "TakeCorpse translation must not publish its terminal Waiting state early"
            );
        }
    }

    #[test]
    fn receive_purse_uses_three_real_sequence_orders_with_stable_identity() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData {
                beggar_scroll_sets: Some(vec![vec![]]),
                ..Default::default()
            },
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut manager, crate::element::Command::ReceivePurse, owner);
        let mut next_id = 200;

        assert_eq!(
            begin_receive_purse(&mut entities, &mut manager, owner, seq_id, 0, &mut next_id),
            BeginResult::Started
        );
        let element = manager.get_element_mut(seq_id, 0).unwrap();
        let mut actual = Vec::new();
        while let Some(order) = element.pop_current_order() {
            actual.push((order.order_type, order.order_id));
        }
        assert_eq!(
            actual.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![
                OrderType::ReceivingPurse,
                OrderType::WaitingWithPurse,
                OrderType::TransitionWaitingWithPurseWaitingUpright,
            ]
        );
        assert_eq!(
            entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_ability
                .order_id,
            Some(actual[0].1)
        );
    }

    #[test]
    fn can_carry_on_shoulders_clear_with_no_obstacles() {
        // No obstacles anywhere → ceiling column is always clear.
        let list = ObstacleList {
            static_obstacles: &[],
            dynamic_obstacles: &[],
            static_active: &[],
        };
        let pos = WorldPoint3D {
            x: 100.0,
            y: 100.0,
            z: 0.0,
        };
        assert!(can_carry_on_shoulders(pos, list));
    }

    #[test]
    fn climb_translation_defers_posture_snap_and_orientation_until_execute_initialization() {
        let mut entities = Entities::new();
        let mut climber = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        climber.element.set_position_map(MapPoint::new(10.0, 20.0));
        climber.element.set_direction_instantly(3);
        let mut helper = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::HelpingToClimb,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        helper.element.set_position_map(MapPoint::new(30.0, 40.0));
        helper.element.set_direction_instantly(10);
        entities.push(Some(Entity::Pc(climber)));
        entities.push(Some(Entity::Pc(helper)));
        let climber_id = entities.id_at_legacy_slot(0).unwrap();
        let helper_id = entities.id_at_legacy_slot(1).unwrap();

        let mut manager = SequenceManager::new();
        let seq_id = manager.launch_element(SequenceElement::new_interaction(
            2,
            Command::ClimbUpOnShoulders,
            Some(climber_id),
            Some(helper_id),
        ));
        manager.element_in_progress(seq_id, 0);
        let mut next_order_id = 1;
        let result = begin_climb_on_shoulders(
            &mut entities,
            &mut manager,
            climber_id,
            helper_id,
            seq_id,
            0,
            &mut next_order_id,
            ObstacleList {
                static_obstacles: &[],
                dynamic_obstacles: &[],
                static_active: &[],
            },
        );
        assert!(matches!(result, ClimbResult::Started));

        let climber = entities.get(climber_id).unwrap();
        assert_eq!(climber.element_data().posture, Posture::Upright);
        assert_eq!(
            climber.element_data().position_map(),
            MapPoint::new(10.0, 20.0)
        );
        assert_eq!(climber.element_data().direction(), 3);
        assert_eq!(i16::from(climber.position_iface().get_direction_goal()), 3);
        assert_eq!(climber.human_data().unwrap().carrier, None);
        let helper = entities.get(helper_id).unwrap();
        assert_eq!(helper.element_data().posture, Posture::HelpingToClimb);
        assert_eq!(helper.element_data().direction(), 10);
        assert_eq!(helper.pc_data().unwrap().carried, None);

        let expected_helper_goal =
            crate::position_interface::vector_to_sector_0_to_15_iso(10.0 - 30.0, 20.0 - 40.0);
        initialize_climb_on_shoulders_relationship(&mut entities, climber_id, helper_id);

        let climber = entities.get(climber_id).unwrap();
        assert_eq!(climber.element_data().posture, Posture::OnShoulders);
        assert_eq!(
            climber.element_data().position_map(),
            MapPoint::new(30.0, 40.0)
        );
        assert_eq!(climber.human_data().unwrap().carrier, Some(helper_id));
        let helper = entities.get(helper_id).unwrap();
        assert_eq!(helper.element_data().posture, Posture::CarryingOnShoulders);
        assert_eq!(helper.element_data().direction(), 10);
        assert_eq!(
            i16::from(helper.position_iface().get_direction_goal()),
            expected_helper_goal
        );
        assert_eq!(helper.pc_data().unwrap().carried, Some(climber_id));
    }

    #[test]
    fn terminal_shoulder_sync_does_not_restore_stale_helper_transition() {
        let mut entities = Entities::new();
        let mut climber = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::OnShoulders,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        climber.element.sprite.last_action = OrderType::ClimbingUpOnShoulders;
        climber.element.sprite.current_frame = 5;
        climber.element.sprite.frame_count = 1;

        let mut helper = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::CarryingOnShoulders,
                ..Default::default()
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        helper.element.sprite.last_action = OrderType::WaitingCarryingOnShoulders;
        helper.element.sprite.current_row = 777;
        helper.element.sprite.current_frame = 0;
        helper.element.sprite.frame_count = u16::MAX;
        let mut helper_conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        helper_conversion[OrderType::TransitionHelpingClimbingUp as usize] = 100;
        helper.element.sprite.conversion = std::sync::Arc::new(helper_conversion);
        let mut helper_scripts = vec![crate::sprite_script::SpriteScript::default(); 116];
        helper_scripts[100].frame_ids = vec![0, 1, 2, 3, 4, 5];
        helper.element.sprite.scripts = std::sync::Arc::new(helper_scripts);

        entities.push(Some(Entity::Pc(helper)));
        entities.push(Some(Entity::Pc(climber)));
        let helper_id = entities.id_at_legacy_slot(0).unwrap();
        let climber_id = entities.id_at_legacy_slot(1).unwrap();
        {
            let helper = entities.get_mut(helper_id).unwrap().pc_data_mut().unwrap();
            helper.carried = Some(climber_id);
            helper.set_live_carried_posture(Posture::OnShoulders);
        }
        entities
            .get_mut(climber_id)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(helper_id);

        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());

        let helper = entities.get(helper_id).unwrap().element_data();
        assert_eq!(
            (
                helper.sprite.last_action,
                helper.sprite.current_row,
                helper.sprite.current_frame,
                helper.sprite.frame_count,
            ),
            (OrderType::WaitingCarryingOnShoulders, 777, 0, u16::MAX),
            "a terminated climber's stale sprite action must not overwrite the helper's new idle"
        );

        {
            let rider = entities.get_mut(climber_id).unwrap().element_data_mut();
            rider.sprite.last_action = OrderType::WaitingOnShoulders;
            rider.sprite.current_frame = 2;
            rider.sprite.frame_count = u16::MAX;
        }
        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());
        let rider = entities.get(climber_id).unwrap().element_data();
        assert_eq!(
            (
                rider.sprite.last_action,
                rider.sprite.current_frame,
                rider.sprite.frame_count,
            ),
            (OrderType::WaitingOnShoulders, 2, u16::MAX),
            "idle shoulder sync must preserve the rider's independently driven PerformAction timer"
        );

        {
            let climber = entities.get_mut(climber_id).unwrap();
            let sprite = &mut climber.element_data_mut().sprite;
            sprite.last_action = OrderType::ClimbingUpOnShoulders;
            sprite.current_frame = 5;
            sprite.frame_count = 1;
            climber.actor_data_mut().unwrap().active_ability.kind =
                Some(AbilityKind::ClimbOnShoulders);
        }
        sync_carried_positions(&mut entities, &crate::profiles::ProfileManager::default());
        let helper = entities.get(helper_id).unwrap().element_data();
        assert_eq!(
            helper.sprite.last_action,
            OrderType::TransitionHelpingClimbingUp,
            "a live climb ability must retain helper-side synchronization"
        );
        assert_eq!(
            (helper.sprite.current_frame, helper.sprite.frame_count),
            (5, 1)
        );
    }
}
