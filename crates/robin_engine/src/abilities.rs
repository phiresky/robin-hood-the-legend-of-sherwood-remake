//! Hero special abilities — carry, tie-up, heal, whistle, listen, trap placement.
//!
//! 1. A `begin_*` function is called when the engine dispatches a
//!    `Command::*` sequence element to an actor.  It validates the actor
//!    and target, then appends the animation order to that element.
//!
//! The engine executes each selected animation and its effects directly in
//! the actor's creation-order slot, releasing entity borrows before callbacks.

use crate::coordinates::MapPoint;
use crate::element::{ActionState, Command, Entity, EntityId, Posture};
use crate::entities::Entities;
use crate::movement::AbilityKind;
use crate::order::{Order, OrderType};
use crate::sequence::{SequenceId, SequenceManager};
#[cfg(test)]
use crate::sprite::MotionState as SpriteMotionState;

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

/// HP restored per bandage.
pub const HEAL_AMOUNT: i16 = 75;

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
    // Player-character take-corpse validity. A body
    // already linked to this PC remains valid: authentic restored states can
    // retain that self-link while the PC is upright, and Original explicitly
    // rejects only a *different* carrier.
    let target_valid = match entities.get(target_id) {
        Some(e) => {
            if !e.is_human() {
                false
            } else {
                let posture = e.element_data().posture();
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
    // Player-character take-corpse validity never reads
    // Carried-actor reference: an authentic restored PC may still be linked to a different
    // carried body while a new TakeCorpse is translated. Translate authors the
    // pickup order anyway, and the first execution replaces that reference.
    let carrier = match entities.get_mut(carrier_id) {
        Some(e) => e,
        None => return BeginResult::Impossible,
    };
    if !carrier.is_pc() || carrier.is_dead() {
        return BeginResult::Impossible;
    }
    let order_id = alloc_order_id(order_id_counter);

    let mut order = Order::new(
        OrderType::TransitionWaitingUprightCarryingCorpse,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.antagonist = Some(target_id);
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
/// the transition to waiting upright while carrying a corpse, immediately
/// before freezing and positioning the body,
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
        .posture();
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
    // The original game assigns the carried-actor reference unconditionally. This intentionally replaces
    // a stale/restored link to another body after the Execute-time validity
    // check has accepted the new target.
    pc.carried = Some(target_id);
    pc.set_live_carried_posture(target_posture);

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

    let mut order = Order::new(
        OrderType::TransitionCarryingCorpseWaitingUpright,
        0.0,
        0.0,
        order_id,
    );
    order.antagonist = Some(carried_id);
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
/// Execute, matching shoulder-climbing animation initialization.
///
/// The order is pushed on the *climber*'s sequence element.  The first
/// Execute later calls [`initialize_climb_on_shoulders_relationship`].
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
                e.is_pc() && !e.is_dead() && e.element_data().posture() == Posture::HelpingToClimb;
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
    let order_id = alloc_order_id(order_id_counter);
    let actor = climber.actor_data_mut().unwrap();
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
    order.antagonist = Some(helper_id);
    order.target_actor = Some(helper_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    ClimbResult::Started
}

/// Apply the one-shot relationship setup performed by Original when the
/// `ClimbingUpOnShoulders` order first executes.
///
/// Player-character translation only appends the order. Its execution arm later
/// links the pair, aims the helper from the climber's pre-snap position, flips
/// both postures, and finally snaps the climber onto the helper. Keeping that
/// ordering matters: sampling the climber after the snap collapses the facing
/// vector to zero.
pub(crate) fn initialize_climb_on_shoulders_relationship(
    engine: &mut crate::engine::EngineInner,
    climber_id: EntityId,
    helper_id: EntityId,
) {
    let climber_pos = engine
        .world
        .entities
        .get(climber_id)
        .unwrap_or_else(|| panic!("climb-on-shoulders climber {climber_id:?} disappeared"))
        .element_data()
        .position_map();
    let helper_pos = engine
        .world
        .entities
        .get(helper_id)
        .unwrap_or_else(|| panic!("climb-on-shoulders helper {helper_id:?} disappeared"))
        .element_data()
        .position_map();
    let helper_facing = crate::position_interface::vector_to_sector_0_to_15_iso(
        climber_pos.x - helper_pos.x,
        climber_pos.y - helper_pos.y,
    );

    {
        let helper = engine
            .world
            .entities
            .get_mut(helper_id)
            .expect("validated climb-on-shoulders helper disappeared during initialization");
        let pc = helper
            .pc_data_mut()
            .expect("climb-on-shoulders helper is not a PC");
        pc.carried = Some(climber_id);
        pc.set_live_carried_posture(Posture::OnShoulders);
    }
    engine
        .world
        .entities
        .get_mut(climber_id)
        .expect("validated climb-on-shoulders climber disappeared during initialization")
        .human_data_mut()
        .expect("climb-on-shoulders climber is not human")
        .carrier = Some(helper_id);
    engine
        .world
        .entities
        .get_mut(helper_id)
        .expect("validated climb-on-shoulders helper disappeared before facing setup")
        .element_data_mut()
        .set_direction_goal(helper_facing);
    {
        engine.set_entity_posture(climber_id, Posture::OnShoulders);
        let climber = engine
            .world
            .entities
            .get_mut(climber_id)
            .expect("validated climb-on-shoulders climber disappeared before posture setup");
        climber
            .actor_data_mut()
            .expect("climb-on-shoulders climber lost actor state")
            .action_state = ActionState::Waiting;
    }
    {
        engine.set_entity_posture(helper_id, Posture::CarryingOnShoulders);
        let helper = engine
            .world
            .entities
            .get_mut(helper_id)
            .expect("validated climb-on-shoulders helper disappeared before posture setup");
        helper
            .actor_data_mut()
            .expect("climb-on-shoulders helper lost actor state")
            .action_state = ActionState::Waiting;
    }
    engine
        .world
        .entities
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
/// happen in the engine's shoulder-dismount completion
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
            if e.element_data().posture() != Posture::OnShoulders {
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
    actor.action_state = ActionState::Waiting;

    // Push the climbing-down animation order on the climber's sequence
    // element.  Direction is locked.
    let mut order = Order::new(OrderType::ClimbingDownFromShoulders, 0.0, 0.0, order_id);
    order.antagonist = Some(carrier_id);
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
            let posture = e.element_data().posture();
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

    let mut order = Order::new(OrderType::Tying, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

/// Start releasing a living tied NPC.
///
/// The original game's command set contains an unused untie value, but the shipped game
/// never translated it into a playable action. The Rust extension reuses the
/// authored tying animation in reverse and applies the posture change only at
/// its `DONE` boundary.
pub fn begin_untie(
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

    let target_valid = entities.get(target_id).is_some_and(|target| {
        target.is_active()
            && target.is_npc()
            && !target.is_dead()
            && target.human_data().is_some()
            && target.element_data().posture() == Posture::Tied
    });
    if !target_valid {
        return BeginResult::Impossible;
    }
    let target_pos = entities
        .get(target_id)
        .expect("validated untie target disappeared")
        .element_data()
        .position_map();

    let Some(actor_entity) = entities.get_mut(actor_id) else {
        return BeginResult::Impossible;
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }
    let Some(_) = actor_entity.actor_data_mut() else {
        return BeginResult::Impossible;
    };
    let order_id = alloc_order_id(order_id_counter);

    let mut order = Order::new(OrderType::Tying, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
    order.target_actor = Some(target_id.index());
    order.antagonist = Some(target_id);
    order.compute_direction = false;
    order.reverse = true;
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
/// fires from the `HealDone` branch in `engine::archery`.
pub fn begin_heal(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    healer_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    // Player-character heal translation only authors the
    // Healing order.  The living/injured/range predicate belongs to the
    // following action initialization, after the element has become selected.
    // In particular, a
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
    // Player-character heal translation queues the healing order
    // without stopping the actor or rewriting its logical action state.

    // Queue the canonical Healing order; owner-local ability dispatch swaps to
    // `OrderType::Eating` when the target is the healer itself.
    let mut order = Order::new(OrderType::Healing, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
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
/// Arms `wait_time = TIME_LISTEN_WAIT` (25) so owner-local dispatch
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
    // Original-game player-character whistle translation only queues
    // the whistling order.  In particular, it does not call `Stop()` or
    // overwrite the actor's logical action state: a PC that was bored or
    // moving keeps that state until the animation reaches its terminal
    // state-reset boundary. Rendering reads the same countdown as execution.
    actor.wait_time = TIME_LISTEN_WAIT;

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
/// directly by the engine's eating execution.
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
    // Player-character eat translation only appends the eating
    // order.  The current path and action state remain authoritative until
    // The eating animation terminates and resets states.

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
/// the engine launches a
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
    // Liveness is deliberately NOT gated here. Human-actor instruction
    // in the original game inserts the HITTING
    // order for the hit command unconditionally — its only branch is the
    // `Think(EVENT_STOP)` poke for a moving NPC antagonist. A dead, tied,
    // netted or carried victim is rejected later, by the HITTING
    // initial sequence-element validation gate
    // whose HIT arm tests whether the target is out of order. This implementation runs that
    // same gate in `EngineInner::tick_pending_hit_init`
    // (`engine/archery.rs`), and the Original's actor is visibly committed to
    // hit command — including the walk-to-wait transition inserted during instruction
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

    let mut order = Order::new(OrderType::Hitting, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
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
/// the engine launches a full-life-points
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

    // Player strangle translation appends the order without revalidating the
    // antagonist. A retained post-seek command can therefore become visible
    // for one frame after its target dies; the first Strangling Execute runs
    // sequence-element validation and aborts it on the following owner slot.
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
    // Player strangle translation only appends the Strangling
    // order.  In particular, a seek that launches its post-seek sequence
    // synchronously retains the movement state until the later posture
    // transition actually changes it.

    let mut order = Order::new(OrderType::Strangling, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    BeginResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Begin — Listen (any PC)
// ═══════════════════════════════════════════════════════════════════

/// Append the listening entry, countdown, and exit orders.
/// The selected order determines which part executes.
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

    // Honour the Listen portrait slot. Original-game disabled action masks are
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
    // The original game's listen transition preserves the current
    // path/action state while queuing all three orders and writes the shared
    // serialized wait timer immediately. The transition animation owns
    // the sprite independently of that logical state.
    actor.wait_time = TIME_LISTEN_WAIT;

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
/// the engine's direct apple-throw execution.
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
        OrderType::ThrowingStone,
    )
}

/// Start a stone-throw animation toward a ground point.
///
/// This deliberately shares [`AbilityKind::ThrowStone`] and
/// [`OrderType::ThrowingStone`] with the entity-targeted action, while a null
/// order antagonist identifies the deterministic ground completion
/// path.
pub fn begin_throw_stone_at_ground(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_pos: MapPoint,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
) -> BeginResult {
    let actor_entity = match entities.get_mut(actor_id) {
        Some(entity) => entity,
        None => return BeginResult::Impossible,
    };
    if actor_entity.is_dead() || !actor_entity.is_pc() {
        return BeginResult::Impossible;
    }

    let order_id = alloc_order_id(order_id_counter);
    let actor = match actor_entity.actor_data_mut() {
        Some(actor) => actor,
        None => return BeginResult::Impossible,
    };
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(
        OrderType::ThrowingStone,
        target_pos.x,
        target_pos.y,
        order_id,
    );
    order.compute_direction = false;
    sequence_manager.push_order_on(seq_id, elem_idx, order);

    let actor_pos = actor_entity.element_data().position_map();
    actor_entity.element_data_mut().set_direction_instantly(
        crate::position_interface::vector_to_sector_0_to_15_iso(
            target_pos.x - actor_pos.x,
            target_pos.y - actor_pos.y,
        ),
    );
    BeginResult::Started
}

/// Shared begin path for entity-targeted throws (apple, stone).  The
/// antagonist entity is stored on the order so the
/// completion handler can compute the target's eyes / center as the
/// trajectory endpoint.
fn begin_throw_at_entity(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    actor_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    order_id_counter: &mut u32,
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
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(order_type, target_pos.x, target_pos.y, order_id);
    order.antagonist = Some(target_id);
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
/// Same as [`begin_throw_net`] — TODO: implement gradual turning.
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
/// Same as [`begin_throw_net`] — TODO: implement gradual turning.
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
    // Purse throwing requires Waiting, but the original game's action transition owns
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
/// On completion, the engine deducts the beggar salary
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
    actor.action_state = ActionState::Waiting;

    let mut order = Order::new(OrderType::Paying, pc_pos.x, pc_pos.y, order_id);
    order.antagonist = Some(beggar_id);
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
/// `WaitingWithPurse` → transition-back-to-upright. The selected order
/// lets owner-local dispatch fire [`EngineInner::reveal_scrolls`]
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
    // The beggar must already be idling in `Waiting` before the
    // purse-chain can begin.  Beggars are stationary NPCs so this is
    // nearly always satisfied; reject the command on anything else
    // (e.g. a still-walking beggar) so the sequence manager can
    // surface it as `Impossible` rather than silently forcing the
    // state.
    if actor.action_state != ActionState::Waiting {
        return BeginResult::Impossible;
    }

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
        AbilityKind::Untie => OrderType::Tying,
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
            "{kind:?} is handled by its selected order — \
             ability_order_type should never be called for it"
        ),
    }
}

/// The selected ability order, borrowed as scalar operands for one execution.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct SelectedAbility {
    pub kind: AbilityKind,
    pub sequence_id: SequenceId,
    pub element_index: usize,
    pub target: Option<EntityId>,
    pub order_id: std::num::NonZeroU32,
    pub order_type: OrderType,
    pub order_done: bool,
}

/// Resolve execution from the actor's selected element and its current order.
pub(crate) fn selected_ability(
    entities: &Entities,
    sequence_manager: &SequenceManager,
    owner: EntityId,
) -> Option<SelectedAbility> {
    let actor = entities.get(owner)?.actor_data()?;
    let reference = actor.selected_sequence_element?;
    let element = sequence_manager
        .get_element(reference.sequence_id, reference.element_index)
        .expect("selected ability element disappeared");
    let order = element.current_order()?;
    let kind = match order.order_type {
        OrderType::TransitionWaitingUprightCarryingCorpse => AbilityKind::Carry,
        OrderType::TransitionCarryingCorpseWaitingUpright => AbilityKind::Drop,
        OrderType::Tying if element.command == Command::Untie => AbilityKind::Untie,
        OrderType::Tying => AbilityKind::Tie,
        OrderType::Healing => AbilityKind::Heal,
        OrderType::Eating if element.command == Command::HealCmd => AbilityKind::Heal,
        OrderType::Eating => AbilityKind::Eat,
        OrderType::Whistling => AbilityKind::Whistle,
        OrderType::TransitionWaitingUprightListening
        | OrderType::Listening
        | OrderType::TransitionListeningWaitingUpright => AbilityKind::Listen,
        OrderType::ThrowingNet => AbilityKind::ThrowNet,
        OrderType::ThrowingWaspNest => AbilityKind::ThrowWaspNest,
        OrderType::ThrowingPurse => AbilityKind::ThrowPurse,
        OrderType::ThrowingApple => AbilityKind::ThrowApple,
        OrderType::ThrowingStone => AbilityKind::ThrowStone,
        OrderType::Paying => AbilityKind::Pay,
        OrderType::ReceivingPurse
        | OrderType::WaitingWithPurse
        | OrderType::TransitionWaitingWithPurseWaitingUpright => AbilityKind::ReceivePurse,
        OrderType::Hitting => AbilityKind::Hit,
        OrderType::Strangling => AbilityKind::Strangle,
        OrderType::ClimbingUpOnShoulders => AbilityKind::ClimbOnShoulders,
        OrderType::ClimbingDownFromShoulders => AbilityKind::ClimbDownFromShoulders,
        _ => return None,
    };
    Some(SelectedAbility {
        kind,
        sequence_id: reference.sequence_id,
        element_index: reference.element_index,
        target: match kind {
            // Exit transitions act on the actor's current carried relationship,
            // independently of the command whose transition is being executed.
            AbilityKind::Drop => entities.get(owner)?.pc_data()?.carried,
            AbilityKind::ClimbDownFromShoulders => entities.get(owner)?.human_data()?.carrier,
            AbilityKind::Heal => match element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
                _ => None,
            },
            _ => order.antagonist.or_else(|| match element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
                _ => None,
            }),
        },
        order_id: order.order_id,
        order_type: order.order_type,
        order_done: order.done,
    })
}

/// Select the carried-body animation set from the carrier's contextual action.
fn uses_little_john_carry(
    profiles: &crate::profiles::ProfileManager,
    profile_index: crate::profiles::CharacterProfileIdx,
) -> bool {
    profiles
        .get_character(profile_index)
        .unwrap_or_else(|| panic!("carrier references missing character profile {profile_index:?}"))
        .contextual_actions
        .contains(&crate::profiles::Action::LittleJohnCarry)
}

/// Synchronize the body immediately after its carrier performs a lift, wait, or drop.
pub(crate) fn sync_corpse_animation_for_carrier(
    entities: &mut Entities,
    profiles: &crate::profiles::ProfileManager,
    carrier_id: EntityId,
    carrier_order: OrderType,
) {
    let carrier = entities
        .get(carrier_id)
        .expect("corpse carrier disappeared");
    let pc = carrier.pc_data().expect("corpse carrier must be a PC");
    let target_id = pc.carried.expect("corpse carrier has no body");
    let little_john = uses_little_john_carry(profiles, pc.profile_index);
    let animation = match (carrier_order, little_john) {
        (OrderType::TransitionWaitingUprightCarryingCorpse, true) => {
            OrderType::BeingLiftedLittleJohn
        }
        (OrderType::TransitionWaitingUprightCarryingCorpse, false) => {
            OrderType::BeingLiftedPeasantC
        }
        (OrderType::TransitionCarryingCorpseWaitingUpright, true) => {
            OrderType::BeingDroppedLittleJohn
        }
        (OrderType::TransitionCarryingCorpseWaitingUpright, false) => {
            OrderType::BeingDroppedPeasantC
        }
        (OrderType::WaitingWithCorpse, true) => OrderType::BeingCarriedLittleJohn,
        (OrderType::WaitingWithCorpse, false) => OrderType::BeingCarriedPeasantC,
        _ => panic!("unsupported corpse synchronization order {carrier_order:?}"),
    };
    let frame = carrier.sprite().current_frame;
    let frame_count = carrier.sprite().frame_count;
    let depth = carrier.sprite().display_depth;
    let target = entities
        .get_mut(target_id)
        .expect("carried body disappeared");
    let direction =
        u16::try_from(target.element_data().direction()).expect("invalid carried direction");
    let sprite = &mut target.element_data_mut().sprite;
    sprite.force_sprite_row(animation, direction);
    sprite.synchronize_anim(frame, frame_count);
    sprite.compute_display_depth_relative_to(depth, false);
}

/// The climber's action drives the frozen helper, including the terminal frame.
pub(crate) fn sync_shoulder_climb_animation(
    entities: &mut Entities,
    climber_id: EntityId,
    climb_order: OrderType,
) {
    let climber = entities
        .get(climber_id)
        .expect("shoulder climber disappeared");
    let helper_id = climber
        .human_data()
        .and_then(|human| human.carrier)
        .expect("shoulder climber has no helper");
    let frame = climber.sprite().current_frame;
    let frame_count = climber.sprite().frame_count;
    let animation = match climb_order {
        OrderType::ClimbingUpOnShoulders => OrderType::TransitionHelpingClimbingUp,
        OrderType::ClimbingDownFromShoulders => OrderType::TransitionHelpingClimbingDown,
        _ => panic!("unsupported shoulder synchronization order {climb_order:?}"),
    };
    let helper = entities
        .get_mut(helper_id)
        .expect("shoulder helper disappeared");
    let direction =
        u16::try_from(helper.element_data().direction()).expect("invalid helper direction");
    let sprite = &mut helper.element_data_mut().sprite;
    sprite.force_sprite_row(animation, direction);
    sprite.synchronize_anim(frame, frame_count);
    let depth = sprite.display_depth;
    let sprite = &mut entities
        .get_mut(climber_id)
        .expect("shoulder climber disappeared")
        .element_data_mut()
        .sprite;
    sprite.compute_display_depth_relative_to(depth, false);
}

/// Walking moves the body on its own surface and resets its carried animation.
pub(crate) fn sync_walking_corpse_for_carrier(
    entities: &mut Entities,
    profiles: &crate::profiles::ProfileManager,
    carrier_id: EntityId,
) {
    let carrier = entities
        .get(carrier_id)
        .expect("walking corpse carrier disappeared");
    let pc = carrier
        .pc_data()
        .expect("walking corpse carrier must be a PC");
    let target_id = pc.carried.expect("walking corpse carrier has no body");
    let position = carrier.element_data().position_map();
    let depth = carrier.sprite().display_depth;
    let direction = carrier.element_data().direction().wrapping_sub(4) & 15;
    let little_john = uses_little_john_carry(profiles, pc.profile_index);
    let target = entities
        .get_mut(target_id)
        .expect("walking carried body disappeared");
    let element = target.element_data_mut();
    element.set_position_map(position);
    element.set_direction_instantly(direction);
    element.sprite.force_animation(
        if little_john {
            OrderType::BeingCarriedLittleJohn
        } else {
            OrderType::BeingCarriedPeasantC
        },
        direction as u16,
    );
    element
        .sprite
        .compute_display_depth_relative_to(depth, false);
}

/// Walking advances the frozen rider's own action after moving the carrier.
pub(crate) fn step_shoulder_rider(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    carrier_id: EntityId,
) {
    let carrier = entities
        .get(carrier_id)
        .expect("walking shoulder carrier disappeared");
    let target_id = carrier
        .pc_data()
        .and_then(|pc| pc.carried)
        .expect("walking shoulder carrier has no rider");
    let position = carrier.element_data().position_map();
    let depth = carrier.sprite().display_depth;
    let direction = (carrier.element_data().direction() + 8) & 15;
    let rider = entities
        .get_mut(target_id)
        .expect("shoulder rider disappeared");
    let element = rider.element_data_mut();
    element.set_position_map(position);
    element.set_direction_instantly(direction);
    element.sprite.perform_action(
        sim,
        None,
        OrderType::WaitingOnShoulders,
        direction as u16,
        crate::sprite::FrameProgression::Default,
        false,
    );
    element
        .sprite
        .compute_display_depth_relative_to(depth, false);
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
    use crate::sprite_script::SpriteScript;

    fn carry_profiles() -> crate::profiles::ProfileManager {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        profiles
    }

    #[test]
    #[should_panic(expected = "carrier references missing character profile")]
    fn carry_style_rejects_missing_character_profile() {
        uses_little_john_carry(
            &crate::profiles::ProfileManager::default(),
            crate::profiles::CharacterProfileIdx(0),
        );
    }

    fn launch_ability_element(
        entities: &mut Entities,
        manager: &mut SequenceManager,
        command: crate::element::Command,
        owner: EntityId,
    ) -> SequenceId {
        let seq_id = manager.insert_element(SequenceElement::new(1, command, Some(owner)));
        manager.start_sequence_level(seq_id);
        manager
            .get_sequence_mut(seq_id)
            .unwrap()
            .increase_elements_in_progress();
        manager.get_element_mut(seq_id, 0).unwrap().state =
            crate::sequence::SequenceState::InProgress;
        manager.rebuild_indices();
        entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(seq_id, 0));
        seq_id
    }

    fn take_corpse_translation_fixture() -> (Entities, EntityId, EntityId, EntityId) {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element =
                    ElementData::from_initial_posture(Posture::CarryingCorpse);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..Default::default()
            },
        })));
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Lying);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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

    #[test]
    fn selected_ability_uses_each_actions_live_target_owner() {
        let (mut entities, owner, carried, command_target) = take_corpse_translation_fixture();
        entities
            .get_mut(owner)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(carried);
        entities
            .get_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(carried);
        let mut manager = SequenceManager::new();
        for (command, action, order_target, expected) in [
            (
                Command::HealCmd,
                OrderType::TransitionCarryingCorpseWaitingUpright,
                Some(command_target),
                carried,
            ),
            (
                Command::Move,
                OrderType::ClimbingDownFromShoulders,
                None,
                carried,
            ),
            (
                Command::HealCmd,
                OrderType::Healing,
                Some(carried),
                command_target,
            ),
            (Command::TieCmd, OrderType::Tying, Some(carried), carried),
        ] {
            let mut element =
                SequenceElement::new_interaction(1, command, Some(owner), Some(command_target));
            let mut order = Order::new(action, 0.0, 0.0, std::num::NonZeroU32::new(1).unwrap());
            order.antagonist = order_target;
            element.push_order(order);
            let sequence = manager.insert_element(element);
            entities
                .get_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .selected_sequence_element =
                Some(crate::sequence::SequenceElementRef::new(sequence, 0));
            assert_eq!(
                selected_ability(&entities, &manager, owner).unwrap().target,
                Some(expected),
                "{command:?}/{action:?}"
            );
        }
    }

    /// Seed3 linux3 Savegame_007 replay-011 and Savegame_008 replay-028:
    /// restored PC 194 is CarryingCorpse/Waiting and remains reciprocally
    /// linked to one body while TakeCorpse targets a second available body.
    /// The original game ignores the carried-actor reference during validity, installs action 188, and
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
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::TakeCorpse,
            carrier,
        );
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
        assert_eq!(
            entities
                .get(old_target)
                .unwrap()
                .human_data()
                .unwrap()
                .carrier,
            Some(carrier),
            "the old body's reciprocal reference is retained here"
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
            let seq_id = launch_ability_element(
                &mut entities,
                &mut manager,
                crate::element::Command::TakeCorpse,
                carrier,
            );
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
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Carried);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element
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
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::BeingCarriedPeasantC as usize] = 100;
        body.element.sprite.conversion = std::sync::Arc::new(conversion);
        let mut scripts = vec![crate::sprite_script::SpriteScript::default(); 116];
        scripts[104].frame_ids = vec![0, 1, 2, 3];
        scripts[104].delays = vec![0; 4];
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
    fn waiting_with_corpse_publishes_idle_carried_visual() {
        let (mut entities, carrier, body) = corpse_carry_fixture(OrderType::WaitingWithCorpse);
        {
            let carrier_sprite = &mut entities.get_mut(carrier).unwrap().element_data_mut().sprite;
            carrier_sprite.current_frame = 2;
            carrier_sprite.frame_count = 7;
        }

        sync_corpse_animation_for_carrier(
            &mut entities,
            &carry_profiles(),
            carrier,
            OrderType::WaitingWithCorpse,
        );

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
            scripts[204].delays = vec![0; 4];
            sprite.scripts = std::sync::Arc::new(scripts);
        }

        sync_corpse_animation_for_carrier(
            &mut entities,
            &carry_profiles(),
            carrier,
            OrderType::TransitionWaitingUprightCarryingCorpse,
        );

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

        sync_walking_corpse_for_carrier(&mut entities, &carry_profiles(), carrier);

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
        sync_corpse_animation_for_carrier(
            &mut entities,
            &carry_profiles(),
            carrier,
            OrderType::TransitionCarryingCorpseWaitingUpright,
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
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element
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
        let seq_id = manager.insert_element(SequenceElement::new_interaction(
            1,
            Command::Pay,
            Some(pc),
            Some(beggar),
        ));
        manager.start_sequence_level(seq_id);
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
    fn untie_translation_targets_living_tied_npc_and_reverses_tying_animation() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..Default::default()
            },
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: HumanData {
                unconscious: true,
                ..Default::default()
            },
            npc: NpcData {
                life_points: 100,
                ..Default::default()
            },
            civilian: CivilianData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = manager.insert_element(SequenceElement::new_interaction(
            1,
            Command::Untie,
            Some(owner),
            Some(target),
        ));
        manager.start_sequence_level(seq_id);
        manager.get_element_mut(seq_id, 0).unwrap().state =
            crate::sequence::SequenceState::InProgress;
        manager.rebuild_indices();
        entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(seq_id, 0));
        let mut next_order_id = 77;

        assert_eq!(
            begin_untie(
                &mut entities,
                &mut manager,
                owner,
                target,
                seq_id,
                0,
                &mut next_order_id,
            ),
            BeginResult::Started
        );

        let ability = selected_ability(&entities, &manager, owner).expect("selected untie order");
        assert_eq!(ability.kind, AbilityKind::Untie);
        assert_eq!(ability.target, Some(target));
        let order = manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap();
        assert_eq!(order.order_type, OrderType::Tying);
        assert_eq!(order.target_actor, Some(target.index()));
        assert_eq!(order.antagonist, Some(target));
        assert!(order.reverse);
        assert!(!order.compute_direction);
    }

    #[test]
    fn untie_plays_tying_frames_backwards_through_terminal_completion() {
        let tying_script = SpriteScript {
            action_id: OrderType::Tying as u16,
            action_done: 2,
            frame_ids: vec![10, 11, 12, 13],
            delays: vec![0; 4],
            distances: vec![0; 4],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
            sound_ids: vec![0; 4],
            ..SpriteScript::default()
        };
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::Tying as usize] = 0;

        let mut owner_element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        };
        owner_element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![tying_script; 16]),
            std::sync::Arc::new(conversion),
        );
        let mut engine = crate::engine::EngineInner::new();
        let owner = engine.add_test_entity(Entity::Pc(ActorPc {
            element: owner_element,
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..Default::default()
            },
        }));
        let target = engine.add_test_entity(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: HumanData {
                unconscious: true,
                concussion_of_the_brain: 100,
                concussion_healing_timeout: 100,
                ..Default::default()
            },
            npc: NpcData::default(),
            civilian: CivilianData::default(),
        }));
        let mut entities = std::mem::take(&mut engine.world.entities);
        let mut manager = SequenceManager::new();
        let seq_id = manager.insert_element(SequenceElement::new_interaction(
            1,
            Command::Untie,
            Some(owner),
            Some(target),
        ));
        manager.start_sequence_level(seq_id);
        manager
            .get_sequence_mut(seq_id)
            .unwrap()
            .increase_elements_in_progress();
        manager.get_element_mut(seq_id, 0).unwrap().state =
            crate::sequence::SequenceState::InProgress;
        manager.rebuild_indices();
        entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(seq_id, 0));
        let mut next_order_id = 1;
        assert_eq!(
            begin_untie(
                &mut entities,
                &mut manager,
                owner,
                target,
                seq_id,
                0,
                &mut next_order_id,
            ),
            BeginResult::Started
        );

        engine.world.entities = entities;
        engine.orders.sequence_manager = manager;
        let mut assets = engine.test_runtime_assets();
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].contextual_actions[0] =
            crate::profiles::Action::Tie;
        engine.control.sim_config.enable_unbinding = true;
        let sim = crate::sim_rng::test_context();
        engine.tick_actor_owner_envelopes(&sim, &assets);
        assert_eq!(
            engine.get_entity(owner).unwrap().sprite().current_frame,
            3,
            "the reverse action must begin on the tying animation's last frame"
        );

        let released = (0..8).any(|_| {
            engine.tick_actor_owner_envelopes(&sim, &assets);
            engine.get_entity(target).unwrap().posture() != Posture::Tied
        });
        assert!(released, "reversed Tying must release the target at DONE");
        assert!(
            engine
                .get_entity(target)
                .unwrap()
                .human_data()
                .unwrap()
                .unconscious
        );
        assert!(
            selected_ability(
                &engine.world.entities,
                &engine.orders.sequence_manager,
                owner
            )
            .is_some(),
            "the release must retain the unfinished reverse animation"
        );

        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .done,
            "the owner envelope must commit the DONE boundary"
        );
        let mut tail_frames = Vec::new();
        let terminated = (0..8).any(|_| {
            engine.tick_actor_owner_envelopes(&sim, &assets);
            tail_frames.push(engine.get_entity(owner).unwrap().sprite().current_frame);
            selected_ability(
                &engine.world.entities,
                &engine.orders.sequence_manager,
                owner,
            )
            .is_none()
        });
        assert!(
            terminated,
            "the release must play the rest of the reverse animation"
        );
        assert!(
            tail_frames.contains(&1),
            "reverse tail must advance past DONE: {tail_frames:?}"
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .element_data()
                .sprite
                .current_frame,
            0
        );
    }

    #[test]
    fn hit_translation_preserves_live_movement_state_and_facing() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
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
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::HitCmd,
            attacker,
        );
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
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Friendly(Box::default()),
                    ..Default::default()
                },
                ..Default::default()
            },
            civilian: CivilianData::default(),
        })));
        let attacker = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        {
            let target = entities.get_mut(target).unwrap();
            target
                .element_data_mut()
                .publish_order_posture(Posture::Dead);
            target.npc_data_mut().unwrap().life_points = 0;
        }
        entities
            .get_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Moving;

        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut entities, &mut manager, Command::StrangleCmd, attacker);
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
                element: {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element
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
        let seq_id = launch_ability_element(&mut entities, &mut manager, Command::HealCmd, healer);
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
                element: {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element
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
            .life_points = crate::pc_status::LIFEPOINTS_PC;

        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(&mut entities, &mut manager, Command::HealCmd, healer);
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
                element: {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element
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
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::HitCmd,
            attacker,
        );
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
        let mut engine = crate::engine::EngineInner::new();
        engine.world.entities = entities;
        engine.orders.sequence_manager = manager;
        let assets = crate::engine::LevelAssets::new();
        let sim = crate::sim_rng::test_context();

        for expected_direction in [2, 3, 4] {
            engine.tick_selected_ability(&sim, &assets, attacker, false);
            let entity = engine.world.entities.get(attacker).unwrap();
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
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        entities.push(Some(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element
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
        let mut manager = SequenceManager::new();
        let seq_id =
            launch_ability_element(&mut entities, &mut manager, Command::StrangleCmd, attacker);
        let mut order = Order::new(
            OrderType::Strangling,
            0.0,
            0.0,
            std::num::NonZeroU32::new(11).unwrap(),
        );
        order.antagonist = Some(victim);
        manager.push_order_on(seq_id, 0, order);
        entities
            .get_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = false;
        let mut engine = crate::engine::EngineInner::new();
        engine.world.entities = entities;
        engine.orders.sequence_manager = manager;
        let assets = crate::engine::LevelAssets::new();
        let sim = crate::sim_rng::test_context();

        for expected_attacker in [2, 4] {
            engine.tick_selected_ability(&sim, &assets, attacker, false);
            assert_eq!(
                engine
                    .world
                    .entities
                    .get(attacker)
                    .unwrap()
                    .element_data()
                    .direction(),
                expected_attacker
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .get(victim)
                    .unwrap()
                    .element_data()
                    .direction(),
                8
            );
        }
        engine.tick_selected_ability(&sim, &assets, attacker, false);
        assert_eq!(
            engine
                .world
                .entities
                .get(attacker)
                .unwrap()
                .element_data()
                .direction(),
            4
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(victim)
                .unwrap()
                .element_data()
                .direction(),
            6
        );
        assert_eq!(
            selected_ability(
                &engine.world.entities,
                &engine.orders.sequence_manager,
                attacker
            )
            .unwrap()
            .sequence_id,
            seq_id,
            "turning must retain the exact owner/sequence/element/order identity",
        );
    }

    #[test]
    fn listen_uses_three_real_sequence_orders_with_stable_identity() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::EnterListen,
            owner,
        );
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
        let mut actual = Vec::new();
        while let Some(ability) = selected_ability(&entities, &manager, owner) {
            actual.push((ability.order_type, ability.order_id));
            manager
                .get_element_mut(seq_id, 0)
                .unwrap()
                .pop_current_order();
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
        assert!(selected_ability(&entities, &manager, owner).is_none());
    }

    #[test]
    fn frozen_listen_entry_returns_in_progress_without_advancing_sprite() {
        let mut entities = Entities::new();
        entities.push(Some(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })));
        let owner = entities.id_at_legacy_slot(0).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::EnterListen,
            owner,
        );
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

        let mut engine = crate::engine::EngineInner::new();
        engine.world.entities = entities;
        engine.orders.sequence_manager = manager;
        let motion = engine.tick_selected_ability(
            &crate::sim_rng::test_context(),
            &crate::engine::LevelAssets::new(),
            owner,
            true,
        );

        let sprite = &engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .sprite;
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
            motion,
            Some(SpriteMotionState::InProgress),
            "frozen Execute returns in-progress independently of the previous sprite edge"
        );
        assert_eq!(
            sprite.last_motion_state,
            Some(SpriteMotionState::Done),
            "a skipped sprite call leaves its previous edge untouched"
        );
    }

    #[test]
    fn carry_creates_only_its_canonical_sequence_order() {
        let mut entities = Entities::new();
        let mut carrier_entity = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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
        let mut carrier_conversion = crate::engine::test_support::unmapped_conversion();
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Dead);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
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
            .set_position_map(MapPoint::new(70.0, 80.0));
        target_entity.element.set_direction_instantly(4);
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::BeingLiftedPeasantC as usize] = 100;
        target_entity.element.sprite.conversion = std::sync::Arc::new(conversion);
        target_entity.element.sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript::default(); 116]);
        entities.push(Some(Entity::Pc(carrier_entity)));
        entities.push(Some(Entity::Pc(target_entity)));
        let carrier = entities.id_at_legacy_slot(0).unwrap();
        let target = entities.id_at_legacy_slot(1).unwrap();
        let mut manager = SequenceManager::new();
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::TakeCorpse,
            carrier,
        );
        manager.get_element_mut(seq_id, 0).unwrap().data =
            crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(target),
            };
        entities
            .get_mut(carrier)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;
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
            "translation before the pickup order's first execution must not publish carrier assignment"
        );
        assert_eq!(
            target_entity.element_data().position_map(),
            MapPoint::new(70.0, 80.0)
        );
        assert_eq!(target_entity.element_data().direction(), 4);

        let mut engine = crate::engine::EngineInner::new();
        engine.world.entities = entities;
        engine.orders.sequence_manager = manager;
        let mut assets = crate::engine::LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(carry_profiles());
        engine.tick_selected_ability(
            &crate::sim_rng::SimulationContext::with_seed(1),
            &assets,
            carrier,
            false,
        );

        let target_entity = engine.world.entities.get(target).unwrap();
        assert_eq!(
            engine
                .world
                .entities
                .get(carrier)
                .unwrap()
                .pc_data()
                .unwrap()
                .carried,
            Some(target)
        );
        assert_eq!(target_entity.human_data().unwrap().carrier, Some(carrier));
        assert_eq!(
            target_entity.element_data().position_map(),
            MapPoint::new(80.0, 90.0)
        );
        assert_eq!(target_entity.element_data().direction(), 5);

        let element = engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, 0)
            .unwrap();
        let order = element.orders.front().expect("canonical Carry order");
        assert_eq!(
            (order.order_type, order.order_id.get()),
            (OrderType::TransitionWaitingUprightCarryingCorpse, 300)
        );
        assert!(element.pop_current_order().is_some());
        assert!(element.pop_current_order().is_none());

        let carrier_entity = engine.world.entities.get_mut(carrier).unwrap();
        carrier_entity
            .element_data_mut()
            .set_position_map(MapPoint::new(120.0, 140.0));
        carrier_entity
            .element_data_mut()
            .set_direction_instantly(12);
        sync_corpse_animation_for_carrier(
            &mut engine.world.entities,
            &carry_profiles(),
            carrier,
            OrderType::TransitionWaitingUprightCarryingCorpse,
        );
        let target_entity = engine.world.entities.get(target).unwrap();
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
                element: {
                    let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element
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
                element: {
                    let mut initial_element = ElementData::from_initial_posture(Posture::Dead);
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element
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
            let seq_id = launch_ability_element(
                &mut entities,
                &mut manager,
                crate::element::Command::TakeCorpse,
                carrier,
            );
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
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element
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
        let seq_id = launch_ability_element(
            &mut entities,
            &mut manager,
            crate::element::Command::ReceivePurse,
            owner,
        );
        let mut next_id = 200;

        assert_eq!(
            begin_receive_purse(&mut entities, &mut manager, owner, seq_id, 0, &mut next_id),
            BeginResult::Started
        );
        let mut actual = Vec::new();
        while let Some(ability) = selected_ability(&entities, &manager, owner) {
            actual.push((ability.order_type, ability.order_id));
            manager
                .get_element_mut(seq_id, 0)
                .unwrap()
                .pop_current_order();
        }
        assert_eq!(
            actual.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![
                OrderType::ReceivingPurse,
                OrderType::WaitingWithPurse,
                OrderType::TransitionWaitingWithPurseWaitingUpright,
            ]
        );
        assert!(selected_ability(&entities, &manager, owner).is_none());
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        climber.element.set_position_map(MapPoint::new(10.0, 20.0));
        climber.element.set_direction_instantly(3);
        let mut helper = ActorPc {
            element: {
                let mut initial_element =
                    ElementData::from_initial_posture(Posture::HelpingToClimb);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
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
        let seq_id = manager.insert_element(SequenceElement::new_interaction(
            2,
            Command::ClimbUpOnShoulders,
            Some(climber_id),
            Some(helper_id),
        ));
        manager.start_sequence_level(seq_id);
        manager.get_element_mut(seq_id, 0).unwrap().state =
            crate::sequence::SequenceState::InProgress;
        manager.rebuild_indices();
        entities
            .get_mut(climber_id)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .selected_sequence_element = Some(crate::sequence::SequenceElementRef::new(seq_id, 0));
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
        assert_eq!(climber.element_data().posture(), Posture::Upright);
        assert_eq!(
            climber.element_data().position_map(),
            MapPoint::new(10.0, 20.0)
        );
        assert_eq!(climber.element_data().direction(), 3);
        assert_eq!(i16::from(climber.position_iface().get_direction_goal()), 3);
        assert_eq!(climber.human_data().unwrap().carrier, None);
        let helper = entities.get(helper_id).unwrap();
        assert_eq!(helper.element_data().posture(), Posture::HelpingToClimb);
        assert_eq!(helper.element_data().direction(), 10);
        assert_eq!(helper.pc_data().unwrap().carried, None);

        let expected_helper_goal =
            crate::position_interface::vector_to_sector_0_to_15_iso(10.0 - 30.0, 20.0 - 40.0);
        let mut engine = crate::engine::EngineInner::new();
        engine.world.entities = entities;
        initialize_climb_on_shoulders_relationship(&mut engine, climber_id, helper_id);
        let entities = &engine.world.entities;

        let climber = entities.get(climber_id).unwrap();
        assert_eq!(climber.element_data().posture(), Posture::OnShoulders);
        assert_eq!(
            climber.element_data().position_map(),
            MapPoint::new(30.0, 40.0)
        );
        assert_eq!(climber.human_data().unwrap().carrier, Some(helper_id));
        let helper = entities.get(helper_id).unwrap();
        assert_eq!(
            helper.element_data().posture(),
            Posture::CarryingOnShoulders
        );
        assert_eq!(helper.element_data().direction(), 10);
        assert_eq!(
            i16::from(helper.position_iface().get_direction_goal()),
            expected_helper_goal
        );
        assert_eq!(helper.pc_data().unwrap().carried, Some(climber_id));
    }

    #[test]
    fn shoulder_animation_sync_obeys_explicit_execution_order() {
        let mut entities = Entities::new();
        let mut climber = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::OnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        climber.element.sprite.last_action = OrderType::ClimbingUpOnShoulders;
        climber.element.sprite.current_frame = 5;
        climber.element.sprite.frame_count = 1;

        let mut helper = ActorPc {
            element: {
                let mut initial_element =
                    ElementData::from_initial_posture(Posture::CarryingOnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        helper.element.sprite.last_action = OrderType::WaitingCarryingOnShoulders;
        helper.element.sprite.current_row = 777;
        helper.element.sprite.current_frame = 0;
        helper.element.sprite.frame_count = u16::MAX;
        let mut helper_conversion = crate::engine::test_support::unmapped_conversion();
        helper_conversion[OrderType::TransitionHelpingClimbingUp as usize] = 100;
        helper.element.sprite.conversion = std::sync::Arc::new(helper_conversion);
        let mut helper_scripts = vec![crate::sprite_script::SpriteScript::default(); 116];
        helper_scripts[100].frame_ids = vec![0, 1, 2, 3, 4, 5];
        helper_scripts[100].delays = vec![0; 6];
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

        sync_shoulder_climb_animation(&mut entities, climber_id, OrderType::ClimbingUpOnShoulders);
        let helper = entities.get(helper_id).unwrap().element_data();
        assert_eq!(
            (
                helper.sprite.current_row,
                helper.sprite.current_frame,
                helper.sprite.frame_count
            ),
            (100, 5, 1),
            "the climber's terminal execution must synchronize its helper immediately"
        );

        {
            let helper = entities.get_mut(helper_id).unwrap().element_data_mut();
            helper.sprite.last_action = OrderType::WaitingCarryingOnShoulders;
            helper.sprite.current_row = 777;
            helper.sprite.current_frame = 0;
            helper.sprite.frame_count = u16::MAX;
        }

        {
            let climber = entities.get_mut(climber_id).unwrap();
            let sprite = &mut climber.element_data_mut().sprite;
            sprite.last_action = OrderType::ClimbingUpOnShoulders;
            sprite.current_frame = 5;
            sprite.frame_count = 1;
        }
        sync_shoulder_climb_animation(&mut entities, climber_id, OrderType::ClimbingUpOnShoulders);
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

    #[test]
    fn walking_shoulder_rider_advances_its_animation_on_its_own_surface() {
        let (mut entities, carrier, rider, _) = take_corpse_translation_fixture();
        entities
            .get_mut(carrier)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(rider);
        entities
            .get_mut(carrier)
            .unwrap()
            .element_data_mut()
            .set_position_map(MapPoint::new(80.0, 90.0));
        entities
            .get_mut(carrier)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(3);
        let rider_element = entities.get_mut(rider).unwrap().element_data_mut();
        rider_element.set_layer(3);
        rider_element.set_sector(Some(
            crate::position_interface::SectorHandle::new(7).unwrap(),
        ));
        rider_element.set_material(crate::element::GameMaterial::Wood);
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::WaitingOnShoulders as usize] = 0;
        let script = SpriteScript {
            action_id: OrderType::WaitingOnShoulders as u16,
            frame_ids: vec![0, 1, 2, 3, 4, 5, 6, 7],
            delays: vec![0; 8],
            distances: vec![0; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
            ..SpriteScript::default()
        };
        rider_element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        let surface = (
            rider_element.layer(),
            rider_element.sector(),
            rider_element.material(),
        );
        let sim = crate::sim_rng::test_context();
        let frames = (0..4)
            .map(|_| {
                step_shoulder_rider(&sim, &mut entities, carrier);
                entities.get(rider).unwrap().sprite().current_frame
            })
            .collect::<Vec<_>>();
        assert!(
            frames.windows(2).all(|pair| pair[1] > pair[0]),
            "walking must advance the rider's animation instead of resetting it: {frames:?}"
        );
        let rider_element = entities.get(rider).unwrap().element_data();
        assert_eq!(rider_element.position_map(), MapPoint::new(80.0, 90.0));
        assert_eq!(rider_element.direction(), 11);
        assert_eq!(
            (
                rider_element.layer(),
                rider_element.sector(),
                rider_element.material()
            ),
            surface
        );
        assert_eq!(
            rider_element.sprite.display_depth,
            entities.get(carrier).unwrap().sprite().display_depth + 0.001
        );
        assert_eq!(rider_element.sprite.display_order_ref, None);
    }

    #[test]
    fn live_shoulder_sync_preserves_progressive_climber_turn() {
        let mut entities = Entities::new();
        let mut helper = ActorPc {
            element: {
                let mut initial_element =
                    ElementData::from_initial_posture(Posture::CarryingOnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        helper.element.set_position_map(MapPoint::new(30.0, 40.0));
        helper.element.set_direction_instantly(3);
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::TransitionHelpingClimbingUp as usize] = 0;
        helper.element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![SpriteScript::default(); 16]),
            std::sync::Arc::new(conversion),
        );

        let mut climber = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::OnShoulders);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        // The climbing Execute has just applied one Turn step from 13 toward
        // the carrier-relative goal 11.
        climber.element.set_direction_instantly(12);
        climber.element.set_direction_goal(11);

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

        let position = entities
            .get(climber_id)
            .unwrap()
            .element_data()
            .position_map();
        sync_shoulder_climb_animation(&mut entities, climber_id, OrderType::ClimbingUpOnShoulders);

        let climber = entities.get(climber_id).unwrap();
        assert_eq!(climber.element_data().position_map(), position);
        assert_eq!(climber.element_data().direction(), 12);
        assert_eq!(i16::from(climber.position_iface().get_direction_goal()), 11);
    }
}
