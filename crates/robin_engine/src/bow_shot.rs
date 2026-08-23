//! Bow-shot execution — dispatch logic for `Command::ShootBow`.
//!
//! Implements the end-to-end flow for firing an arrow at a target:
//!
//! 1. [`begin_bow_shot`] is called by the engine when a
//!    `Command::ShootBow` sequence element is dispatched to a shooter.
//!    It sets the shooter into the appropriate aiming action state,
//!    pushes aim-transition and shoot orders onto the order queue,
//!    and marks the `ActiveShot` in-progress.
//!
//! 2. [`tick_bow_shots`] runs every engine tick and, for each actor with an
//!    [`ActiveShot`], drives the sprite through transition animations
//!    and the shoot animation.  On the frame the shoot animation reports
//!    [`SpriteMotionState::Done`], the tick returns a
//!    [`ShotTickResult`] for each completed shot so the engine layer
//!    can compute the trajectory and spawn the arrow.
//!
//! 3. The engine layer (`EngineInner::tick_bow_shots`) receives the result,
//!    looks up the shooter's bow profile, rolls the hit chance,
//!    computes a ballistic trajectory via [`compute_initial_throw_velocity`]
//!    and [`compute_trajectory_ballistic`], and spawns the arrow via
//!    [`spawn_arrow`].
//!
//! 4. [`tick_arrows`] runs every engine tick and advances each arrow
//!    along its precomputed ballistic trajectory (popping waypoints
//!    from the trajectory list, interpolating between them).  When the
//!    arrow comes within [`HIT_DISTANCE`] of any human, or the
//!    trajectory runs out, it returns an [`ArrowTickResult`].  The
//!    engine layer turns that into a `ReceiveArrowDamage` sequence
//!    element so damage and death animations follow the normal
//!    sequence-manager path.
//!
//! ## UI action-slot refresh
//!
//! When ammo reaches 0, the ammo decrement path
//! (`engine/combat.rs::decrement_bow_ammo`) calls
//! `EngineInner::disable_pc_action`, which resolves Bow through the
//! PC's profile action list and sets that portrait slot in
//! `PcData::disabled_actions`.  The HUD action-slot strip is
//! immediate-mode (see `ui_panel.rs`) and re-reads `disabled_actions`
//! each frame, so no messenger notification is needed; the next frame
//! shows the disabled bow slot automatically.

use crate::combat::{self, ConcussionContext};
use crate::coordinates::{GroundPoint, MapPoint, WorldPoint3D, WorldVec3D};
use crate::element::{
    ActionState, Animation, Command, ElementData, ElementKind, ElementProjectile, Entity, EntityId,
    ObjectData, ObjectType, Posture, ProjectileData, TargetFilter, TrajectoryPoint,
};
use crate::entities::{Entities, EntitySlots};
use crate::movement::ActiveShot;
use crate::order::{Order, OrderType};
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::profiles::{Action, ProfileManager};
use crate::sequence::{SequenceElement, SequenceElementData, SequenceId, SequenceManager};
use crate::sprite::MotionState as SpriteMotionState;
use crate::weapons::ShootMode;

mod collision;
mod projectile;
mod trajectory;

pub use collision::*;
pub use projectile::*;
pub use trajectory::*;

#[cfg(test)]
use projectile::preserve_falling_hole_disappearance;
#[cfg(test)]
use trajectory::{
    bow_miss_skill_factor, compute_trajectory_ballistic_bounce_with_terminal,
    projectile_impact_ratio,
};
use trajectory::{
    compute_trajectory_ballistic_impl, compute_trajectory_ballistic_with_terminal_metadata,
};

// ═══════════════════════════════════════════════════════════════════
//  Physics constants
// ═══════════════════════════════════════════════════════════════════

/// Gravitational acceleration (negative = downward).
pub const GRAVITY: f32 = -8.01;

/// Arrow mass for flat (normal / down) shots.
pub const MASS_ARROW_FLAT: f32 = 0.1;

/// Arrow mass for high (long) shots — heavier for a steeper arc.
pub const MASS_ARROW_HIGH: f32 = 0.9;

// Throwable projectile masses.
pub const MASS_APPLE: f32 = 0.8;
pub const MASS_PURSE: f32 = 0.2;
pub const MASS_WASP_NEST: f32 = 0.5;
pub const MASS_NET: f32 = 0.6;
pub const MASS_STONE: f32 = 0.1;

// Throwable apex heights.
pub const APEX_APPLE: f32 = 15.0;
pub const APEX_PURSE: f32 = 15.0;
pub const APEX_WASP_NEST: f32 = 50.0;
pub const APEX_NET: f32 = 30.0;
// Stone is thrown with flight_time = 1 (see `spawn_stone`), so
// `compute_initial_throw_velocity` takes the `v = 0.5 * direction` branch
// and the apex value is never consulted.  Keep at 0.001 to preserve
// replay determinism if the flight-time path ever changes.
pub const APEX_STONE: f32 = 0.001;

/// Number of game frames per trajectory segment.
pub const TIME_FLYSEGMENT: u16 = 4;

/// Distance (map units) at which an arrow can hit a victim.
pub const HIT_DISTANCE: f32 = 15.0;

/// Experience points awarded for a bow kill.
pub const BOW_KILL_EXPERIENCE_POINTS: u32 = 20;

fn set_projectile_animation(proj: &mut ElementProjectile, animation: Animation) {
    proj.object.animation = animation;
    if proj.element.sprite.current_conversion().is_empty() {
        return;
    }
    assert!(
        proj.element.sprite.has_animation(animation),
        "projectile {:?} is missing required animation {animation:?}",
        proj.object.object_type
    );
    // Original Apple/Stone HitObstacle/HitHuman/HitTarget use the
    // directionless ForceAnimation overload, whose default direction is 0.
    proj.element.sprite.force_animation(animation, 0);
}

/// Z offset added to the bow point for long (high) shots.
const BOW_Z_OFFSET_LONG: f32 = 50.0;

/// Z offset added to the bow point for normal (flat) shots.
const BOW_Z_OFFSET_NORMAL: f32 = 40.0;

// Sprite order ids for bow shots are allocated from `EngineInner::next_order_id`
// (passed in by the caller as `&mut u32`) so rollback / replay reproduces
// the same id sequence.

// ═══════════════════════════════════════════════════════════════════
//  Shoot-mode helpers
// ═══════════════════════════════════════════════════════════════════

/// Determine the shoot mode from the shooter's current action state.
pub fn shoot_mode_from_action_state(state: ActionState) -> ShootMode {
    match state {
        ActionState::AimingWithBowUp => ShootMode::Long,
        ActionState::AimingWithBowDown => ShootMode::Down,
        _ => ShootMode::Normal,
    }
}

/// Whether the shot uses a flat trajectory (low mass, fast).
pub fn is_flat_shot(mode: ShootMode) -> bool {
    matches!(mode, ShootMode::Normal | ShootMode::Down)
}

/// Arrow mass for the given shoot mode.
pub fn arrow_mass(mode: ShootMode) -> f32 {
    if is_flat_shot(mode) {
        MASS_ARROW_FLAT
    } else {
        MASS_ARROW_HIGH
    }
}

/// Determine the appropriate `OrderType` for the shoot animation.
fn shoot_order_type_for_mode(mode: ShootMode, anonymous: bool) -> OrderType {
    match (mode, anonymous) {
        (ShootMode::Normal, true) => OrderType::ShootingWithBowAnonymous,
        (ShootMode::Long, true) => OrderType::ShootingWithBowUpAnonymous,
        (ShootMode::Normal, false) => OrderType::ShootingWithBow,
        (ShootMode::Long, false) => OrderType::ShootingWithBowUp,
        (ShootMode::Down, _) => OrderType::ShootingWithBowLeaningOut,
    }
}

/// Recover the authored shoot mode from a concrete shooting order. Used when
/// rebuilding Rust's derived active-shot latch after loading an Original save.
pub(crate) fn shoot_mode_for_order(order: OrderType) -> Option<ShootMode> {
    match order {
        OrderType::ShootingWithBow | OrderType::ShootingWithBowAnonymous => Some(ShootMode::Normal),
        OrderType::ShootingWithBowUp | OrderType::ShootingWithBowUpAnonymous => {
            Some(ShootMode::Long)
        }
        OrderType::ShootingWithBowLeaningOut => Some(ShootMode::Down),
        _ => None,
    }
}

/// C++ `RHElementActorHuman::ComputeBowPoint` selects these non-anonymous
/// animation ids for hotspot lookup even when the active shoot animation is
/// an anonymous archer variant.
pub(crate) fn bow_point_order_type_for_mode(mode: ShootMode) -> OrderType {
    match mode {
        ShootMode::Normal => OrderType::ShootingWithBow,
        ShootMode::Long => OrderType::ShootingWithBowUp,
        ShootMode::Down => OrderType::ShootingWithBowLeaningOut,
    }
}

/// Absolute projected bow hotspot used by C++ `ComputeBowPoint`:
/// `GetPositionSprite() + sprite.GetPoint(shoot_animation, direction)`.
pub(crate) fn bow_sprite_hand_point(
    entity: &Entity,
    mode: ShootMode,
    direction: i16,
) -> Option<MapPoint> {
    let dir = u16::try_from(direction).ok()?;
    let sprite_pos = entity.cxx_position_sprite();
    let offset = entity
        .element_data()
        .sprite
        .get_point(bow_point_order_type_for_mode(mode), dir)?;
    Some(MapPoint::new(
        sprite_pos.x + offset.x,
        sprite_pos.y + offset.y,
    ))
}

/// Canonical order set accepted by the selected active-bow owner.
pub(crate) const ACTIVE_BOW_ORDERS: &[OrderType] = &[
    OrderType::ShootingWithBow,
    OrderType::ShootingWithBowUp,
    OrderType::ShootingWithBowLeaningOut,
    OrderType::ShootingWithBowAnonymous,
    OrderType::ShootingWithBowUpAnonymous,
    OrderType::TransitionEquipBow,
    OrderType::TransitionRaisingBow,
    OrderType::TransitionLoweringBow,
    OrderType::TransitionRaisingBowLeaningOut,
    OrderType::TransitionLoweringBowLeaningOut,
    OrderType::TransitionLoadingBow,
    OrderType::TransitionUnloadBow,
    OrderType::TransitionUnequipBow,
    OrderType::TransitionEquipBowAnonymous,
    OrderType::TransitionRaisingBowAnonymous,
    OrderType::TransitionLoweringBowAnonymous,
    OrderType::TransitionLoadingBowAnonymous,
    OrderType::TransitionUnloadBowAnonymous,
    OrderType::TransitionUnequipBowAnonymous,
];

fn is_shoot_order(ot: OrderType) -> bool {
    matches!(
        ot,
        OrderType::ShootingWithBow
            | OrderType::ShootingWithBowUp
            | OrderType::ShootingWithBowLeaningOut
            | OrderType::ShootingWithBowAnonymous
            | OrderType::ShootingWithBowUpAnonymous
    )
}

/// Whether this order type is a bow transition animation.
fn is_bow_transition_order(ot: OrderType) -> bool {
    matches!(
        ot,
        OrderType::TransitionEquipBow
            | OrderType::TransitionRaisingBow
            | OrderType::TransitionLoweringBow
            | OrderType::TransitionRaisingBowLeaningOut
            | OrderType::TransitionLoweringBowLeaningOut
            | OrderType::TransitionLoadingBow
            | OrderType::TransitionUnloadBow
            | OrderType::TransitionUnequipBow
            | OrderType::TransitionEquipBowAnonymous
            | OrderType::TransitionRaisingBowAnonymous
            | OrderType::TransitionLoweringBowAnonymous
            | OrderType::TransitionLoadingBowAnonymous
            | OrderType::TransitionUnloadBowAnonymous
            | OrderType::TransitionUnequipBowAnonymous
    )
}

pub(crate) fn is_active_bow_order(ot: OrderType) -> bool {
    ACTIVE_BOW_ORDERS.contains(&ot)
}

fn has_active_bow_order(element: &crate::sequence::SequenceElement) -> bool {
    element
        .orders
        .iter()
        .any(|order| is_active_bow_order(order.order_type))
}

fn apply_bow_transition_state_side_effect(
    entity: &mut Entity,
    order_type: OrderType,
    motion: SpriteMotionState,
) {
    let action_state = match order_type {
        OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
            if motion == SpriteMotionState::Start =>
        {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionLoweringBow | OrderType::TransitionLoweringBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionRaisingBow | OrderType::TransitionRaisingBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            Some(ActionState::AimingWithBowUp)
        }
        OrderType::TransitionLoweringBowLeaningOut
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            entity.element_data_mut().posture = Posture::LeaningOut;
            Some(ActionState::AimingWithBowDown)
        }
        OrderType::TransitionRaisingBowLeaningOut
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            entity.element_data_mut().posture = Posture::Upright;
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Start | SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }
            Some(ActionState::Waiting)
        }
        OrderType::TransitionUnloadBow | OrderType::TransitionUnloadBowAnonymous
            if motion == SpriteMotionState::Start =>
        {
            Some(ActionState::Waiting)
        }
        _ => None,
    };

    if let Some(action_state) = action_state
        && let Some(actor) = entity.actor_data_mut()
    {
        actor.action_state = action_state;
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Aim transition helpers
// ═══════════════════════════════════════════════════════════════════

/// Compute the transition orders needed to move from the current aim
/// state to the desired shoot mode.
///
/// Returns 0–2 transition `OrderType`s that should be pushed before the
/// shoot animation order.
fn aim_transition_orders(
    current_state: ActionState,
    desired_mode: ShootMode,
    anonymous: bool,
) -> Vec<OrderType> {
    let mut transitions = Vec::new();

    match desired_mode {
        ShootMode::Normal => {
            match current_state {
                ActionState::AimingWithBowUp => {
                    // Lower from up → normal
                    transitions.push(if anonymous {
                        OrderType::TransitionLoweringBowAnonymous
                    } else {
                        OrderType::TransitionLoweringBow
                    });
                }
                ActionState::AimingWithBowDown => {
                    // Raise from leaning-out → normal
                    transitions.push(OrderType::TransitionRaisingBowLeaningOut);
                }
                _ => {} // Already in correct position or first shot
            }
        }
        ShootMode::Long => {
            match current_state {
                ActionState::AimingWithBow => {
                    // Raise from normal → up
                    transitions.push(if anonymous {
                        OrderType::TransitionRaisingBowAnonymous
                    } else {
                        OrderType::TransitionRaisingBow
                    });
                }
                ActionState::AimingWithBowDown => {
                    // Raise from leaning-out → normal → up
                    transitions.push(OrderType::TransitionRaisingBowLeaningOut);
                    transitions.push(if anonymous {
                        OrderType::TransitionRaisingBowAnonymous
                    } else {
                        OrderType::TransitionRaisingBow
                    });
                }
                _ => {} // Already up or first shot
            }
        }
        ShootMode::Down => {
            match current_state {
                ActionState::AimingWithBow => {
                    // Lower to leaning-out
                    transitions.push(OrderType::TransitionLoweringBowLeaningOut);
                }
                ActionState::AimingWithBowUp => {
                    // Lower from up → normal → leaning-out
                    transitions.push(if anonymous {
                        OrderType::TransitionLoweringBowAnonymous
                    } else {
                        OrderType::TransitionLoweringBow
                    });
                    transitions.push(OrderType::TransitionLoweringBowLeaningOut);
                }
                _ => {} // Already down or first shot
            }
        }
    }

    transitions
}

fn bow_target_ground_position(entity: &Entity) -> MapPoint {
    if entity.is_fx_target() {
        entity
            .compute_target_center()
            .map(|pos| MapPoint { x: pos.x, y: pos.y })
            .unwrap_or_else(|| {
                let pos = entity.element_data().position();
                MapPoint { x: pos.x, y: pos.y }
            })
    } else if entity.is_human() {
        entity
            .compute_belt_point()
            .map(|pos| MapPoint { x: pos.x, y: pos.y })
            .unwrap_or_else(|| {
                let pos = entity.element_data().position();
                MapPoint { x: pos.x, y: pos.y }
            })
    } else {
        let pos = entity.element_data().position();
        MapPoint { x: pos.x, y: pos.y }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public dispatch
// ═══════════════════════════════════════════════════════════════════

/// Outcome of attempting to start a bow shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginShotResult {
    /// Shooter will play the shoot animation; arrow will spawn on the
    /// action-done frame.  The sequence element is now `InProgress`.
    Started,
    /// Shooter or target no longer valid (despawned or wrong kind).
    /// The sequence element should be marked `Impossible`.
    Impossible,
}

/// Forget Rust's execution-side bow latch before retranslating the same
/// postponed Original sequence element.
///
/// C++ stores the active shot entirely in the selected sequence element and
/// its current order. When an injury postpones that element, resuming it calls
/// `Instruct`/`Translate` again with no separate "shot already active" state.
/// Rust needs the separate [`ActiveShot`] driver while an order is executing,
/// but that driver must not reject re-instruction of its own postponed owner.
pub(crate) fn clear_matching_retranslated_shot(
    entities: &mut Entities,
    owner: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
) {
    let actor = entities
        .get_mut(owner)
        .unwrap_or_else(|| panic!("postponed bow owner {owner:?} disappeared"))
        .actor_data_mut()
        .unwrap_or_else(|| panic!("postponed bow owner {owner:?} has no actor data"));
    if actor.active_shot.sequence_id == Some(seq_id) && actor.active_shot.element_index == elem_idx
    {
        actor.active_shot.clear();
    }
}

/// Begin a bow shot on behalf of a `Command::ShootBow` sequence element.
///
/// Called from the engine's sequence-action dispatch when it sees a
/// `Command::ShootBow` instruction for an actor owner.
///
/// The function determines the required shoot mode based on target
/// distance, inserts any necessary aim-transition orders, the shoot
/// animation order, and a reload/unequip order after the shot.
///
/// Returns [`BeginShotResult::Started`] if the shooter has been queued
/// to play the shoot animation; [`BeginShotResult::Impossible`] if the
/// shooter or target is not in a valid state.
#[allow(clippy::too_many_arguments)]
pub fn begin_bow_shot(
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    shooter_id: EntityId,
    target_id: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    shoot_once: bool,
    ammo_count: u32,
    // Shoot mode determined by the engine via `can_shoot_with_bow_at`.
    // `None` means the engine couldn't determine a mode; falls back
    // to the current action state or Normal.
    resolved_shoot_mode: Option<ShootMode>,
    next_order_id: &mut u32,
) -> BeginShotResult {
    // Validate target: it must exist, be shootable, and not be the shooter.
    // Original `CanShootWithBowAt(const RHElement*)` does not test IsActive:
    // `RHEngine::RemoveElement` deliberately retains removed objects because
    // sequence elements may still reference them. A human can therefore go
    // inactive while an injury postpones a shot, and the resumed shot still
    // uses the retained actor's body point.
    if shooter_id == target_id {
        return BeginShotResult::Impossible;
    }
    let target_valid = match entities.get(target_id) {
        Some(e) if e.is_human() => true,
        Some(Entity::Target(t)) => {
            t.element.active && t.target.action_filter.contains(TargetFilter::ARROW)
        }
        None => false,
        Some(_) => false,
    };
    if !target_valid {
        return BeginShotResult::Impossible;
    }

    // Read target ground position for direction/order selection.
    let (tx, ty) = match entities.get(target_id) {
        Some(e) => {
            let position = bow_target_ground_position(e);
            (position.x, position.y)
        }
        None => return BeginShotResult::Impossible,
    };

    // Validate shooter.  Read posture before the mutable borrow.
    let (shooter_valid, shooter_posture, current_state) = match entities.get(shooter_id) {
        Some(e) if e.is_human() && !e.is_dead() => {
            let posture = e.element_data().posture;
            let Some(actor) = e.actor_data() else {
                tracing::warn!(
                    shooter = ?shooter_id,
                    "Begin bow shot rejected: human shooter missing actor data"
                );
                return BeginShotResult::Impossible;
            };
            if actor.active_shot.is_active() {
                (false, posture, ActionState::Waiting)
            } else {
                (true, posture, actor.action_state)
            }
        }
        _ => return BeginShotResult::Impossible,
    };
    if !shooter_valid {
        return BeginShotResult::Impossible;
    }

    let shooter = match entities.get_mut(shooter_id) {
        Some(e) => e,
        None => panic!("validated bow shooter {shooter_id:?} disappeared before translation"),
    };
    let actor = match shooter.actor_data_mut() {
        Some(a) => a,
        None => panic!("validated bow shooter {shooter_id:?} lost actor data before translation"),
    };

    // C++ `RHElementActorHuman::Translate(RHCOMMAND_SHOOT_BOW)` chooses
    // raise/lower setup from the sequence element's
    // ActionStateAfterTransition, not from the actor's live state.  That
    // matters when the same element already queued equip/load orders:
    // the live state is still Waiting, but the shoot body must see
    // AimingWithBow and add a raise order for a first long shot.
    let action_state_after_transition = sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|elem| elem.action_state_after_transition)
        .unwrap_or_else(|| {
            panic!("bow shot translation lost sequence element {seq_id:?}[{elem_idx}]")
        });

    // Determine the desired shoot mode.  The engine resolves the mode
    // up front via `can_shoot_with_bow_at` and passes it in; we
    // override for leaning-out, then fall back to the post-transition
    // bow attitude or the current action state.
    let desired_mode = if shooter_posture == Posture::LeaningOut {
        ShootMode::Down
    } else if let Some(mode) = resolved_shoot_mode {
        mode
    } else if action_state_after_transition.is_bow() {
        shoot_mode_from_action_state(action_state_after_transition)
    } else if current_state.is_bow() {
        shoot_mode_from_action_state(current_state)
    } else {
        ShootMode::Normal
    };

    let order_id = crate::order::alloc_order_id(next_order_id);
    actor.active_shot = ActiveShot {
        sequence_id: Some(seq_id),
        element_index: elem_idx,
        target: Some(target_id),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(desired_mode),
    };
    actor.clear_path();

    // Push aim-transition orders if needed.  Orders live on the owning
    // `SequenceElement.orders` — when the element is cancelled, its
    // orders go with it.
    let _ = actor; // actor mutable borrow ends here; below we borrow sequence_manager instead
    let anonymous = shooter_posture == Posture::AnonymousArcher;
    let transitions = aim_transition_orders(action_state_after_transition, desired_mode, anonymous);
    for t in &transitions {
        let mut order = Order::new(*t, tx, ty, crate::order::alloc_order_id(next_order_id));
        order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }

    // Push the shoot animation order.
    let shoot_ot = shoot_order_type_for_mode(desired_mode, anonymous);
    let mut order = Order::new(shoot_ot, tx, ty, order_id);
    order.target_actor = Some(target_id.index());
    order.compute_direction = false;

    sequence_manager.push_order_on(seq_id, elem_idx, order);

    // Push reload or unequip order after the shot.
    // If ammo > 1 and not a one-shot command → LOADING_BOW, else UNEQUIP_BOW.
    if ammo_count > 1 && !shoot_once {
        // Reload — keep aiming.  Anonymous archers use the anonymous
        // variant of the transition.
        let reload_ot = if shooter_posture == Posture::AnonymousArcher {
            OrderType::TransitionLoadingBowAnonymous
        } else {
            OrderType::TransitionLoadingBow
        };
        let mut reload_order = Order::new(
            reload_ot,
            tx,
            ty,
            crate::order::alloc_order_id(next_order_id),
        );
        reload_order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, reload_order);

        // DownShoot needs an extra lowering transition after reload.
        if desired_mode == ShootMode::Down {
            let mut lower = Order::new(
                OrderType::TransitionLoweringBowLeaningOut,
                tx,
                ty,
                crate::order::alloc_order_id(next_order_id),
            );
            lower.compute_direction = false;
            sequence_manager.push_order_on(seq_id, elem_idx, lower);
        }
    } else {
        // Unequip — last arrow or one-shot command.  Anonymous archers
        // use the anonymous variant of the transition.
        let unequip_ot = if shooter_posture == Posture::AnonymousArcher {
            OrderType::TransitionUnequipBowAnonymous
        } else {
            OrderType::TransitionUnequipBow
        };
        let mut unequip_order = Order::new(
            unequip_ot,
            tx,
            ty,
            crate::order::alloc_order_id(next_order_id),
        );
        unequip_order.compute_direction = false;
        sequence_manager.push_order_on(seq_id, elem_idx, unequip_order);
    }

    BeginShotResult::Started
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame bow-shot tick
// ═══════════════════════════════════════════════════════════════════

/// Per-actor outcome returned from the bow-shot tick.  The engine uses
/// this to compute the trajectory, spawn the arrow, and notify the
/// sequence manager *after* the mutable borrow on `entities` is released.
pub struct ShotTickResult {
    pub shooter: EntityId,
    pub target: EntityId,
    pub seq_id: SequenceId,
    pub elem_idx: usize,
    /// Shooter's 3D entity position (`.z` = ground elevation).
    pub shooter_position: WorldPoint3D,
    /// Target's 2D map position (for arrow direction / spawn).
    pub target_pos: MapPoint,
    /// Target's 3D belt point (for trajectory computation).
    pub target_point: WorldPoint3D,
    /// Shoot mode selected when the command was translated.
    pub shoot_mode: ShootMode,
    /// Shooter's facing direction (0–15) for bow-point computation.
    pub shooter_direction: i16,
    /// Projected sprite hand anchor point (sprite position + hotspot).
    pub sprite_hand_point: MapPoint,
    /// Target's forecasted movement vector for leading shots.
    pub target_forecasted_movement: WorldVec3D,
}

struct PendingShotTickResult {
    shooter: EntityId,
    target: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
    shooter_position: WorldPoint3D,
    shoot_mode: ShootMode,
    shooter_direction: i16,
    sprite_hand_point: MapPoint,
}

#[derive(Default)]
pub struct BowTickEvents {
    pub fired: Vec<ShotTickResult>,
    pub completed: Vec<(SequenceId, usize)>,
    /// Non-script PC equip transitions that reached START while owned by the
    /// specialized active-shot runner. The engine closes Original's
    /// synchronous `MSG_SELECT_ACTION(BOW)` boundary for these owners.
    pub pc_equip_actions: Vec<EntityId>,
}

#[cfg(test)]
thread_local! {
    static CROSS_ACTOR_SHOT_REPLACEMENT: std::cell::Cell<Option<(EntityId, ActiveShot)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn apply_cross_actor_shot_replacement(entities: &mut Entities) {
    let Some((actor_id, replacement)) = CROSS_ACTOR_SHOT_REPLACEMENT.take() else {
        return;
    };
    entities
        .get_mut(actor_id)
        .and_then(Entity::actor_data_mut)
        .expect("cross-actor replacement target must remain an actor")
        .active_shot = replacement;
}

/// Advance the shoot animation for every actor with an [`ActiveShot`].
///
/// Returns a list of results for actors whose shoot animation reached
/// `MotionState::Done` this frame — the engine computes the trajectory,
/// spawns an arrow, and notifies the sequence manager for each.  When
/// the shoot animation completes the actor returns to the
/// AimingWithBow action state.
pub fn tick_bow_shots(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
) -> BowTickEvents {
    tick_bow_shots_matching(sim, entities, sequence_manager, None, None, false)
}

pub fn tick_bow_shot_for_owner(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    owner: EntityId,
    expected_order_id: std::num::NonZeroU32,
    sprite_frozen: bool,
) -> BowTickEvents {
    tick_bow_shots_matching(
        sim,
        entities,
        sequence_manager,
        Some(owner),
        Some(expected_order_id),
        sprite_frozen,
    )
}

fn tick_bow_shots_matching(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sequence_manager: &mut SequenceManager,
    only_owner: Option<EntityId>,
    expected_order_id: Option<std::num::NonZeroU32>,
    sprite_frozen: bool,
) -> BowTickEvents {
    let mut events = BowTickEvents::default();
    let mut pending_fired = Vec::new();
    let mut target_ground_positions = EntitySlots::filled(entities.len(), None);
    let mut target_map_positions = EntitySlots::filled(entities.len(), None);
    for (entity_id, entity) in entities.occupied() {
        target_ground_positions[entity_id] = Some(bow_target_ground_position(entity));
        target_map_positions[entity_id] = Some(entity.element_data().position_map());
    }

    #[cfg(test)]
    apply_cross_actor_shot_replacement(entities);

    for (actor_id, entity) in entities.actors_mut() {
        let shooter_id: EntityId = actor_id.into();
        if only_owner.is_some_and(|owner| owner != shooter_id) {
            continue;
        }
        let actor = match entity.actor_data() {
            Some(a) => a,
            None => continue,
        };
        if !actor.active_shot.is_active() {
            continue;
        }
        let shot = actor.active_shot;
        let order_initialising = actor.execute_order_initialising;
        let direction = entity.element_data().direction();
        // Build 3D shooter position: X/Y from the live map position,
        // Z from the position interface's elevation.
        // The element_data().position field is a dead field; the live
        // ground position is in position_map.
        let elevation = entity.position_iface().get_elevation();
        let shooter_position = WorldPoint3D {
            x: entity.element_data().position_map().x,
            y: entity.element_data().position_map().y,
            z: elevation,
        };
        let _action_state = actor.action_state;

        // Peek at the current order to determine what animation to drive.
        // Orders live on the owning `SequenceElement.orders` (looked up via
        // the `active_shot` handle), not `actor.order_queue`.
        let (shot_seq_id, shot_elem_idx) = match (shot.sequence_id, shot.element_index) {
            (Some(id), ix) => (id, ix),
            _ => continue,
        };
        let (current_order_type, current_order_id, script_driven, bow_order_pending) =
            match sequence_manager
                .get_element(shot_seq_id, shot_elem_idx)
                .and_then(|element| {
                    element.current_order().map(|order| {
                        (
                            order.order_type,
                            order.order_id,
                            element.script_driven,
                            has_active_bow_order(element),
                        )
                    })
                }) {
                Some((order_type, order_id, script_driven, bow_order_pending)) => {
                    (order_type, Some(order_id), script_driven, bow_order_pending)
                }
                None => continue,
            };
        if expected_order_id.is_some() && expected_order_id != current_order_id {
            continue;
        }
        if !is_active_bow_order(current_order_type) {
            if bow_order_pending {
                // C++ `Translate(SHOOT_BOW)` appends bow orders after
                // any pre-command setup transitions already owned by the
                // same sequence element. The active shot has been
                // registered, but the actor must finish those setup orders
                // before the bow runner starts driving the shoot body.
                continue;
            }
            tracing::debug!(
                shooter = shooter_id.index(),
                ?shot_seq_id,
                shot_elem_idx,
                ?current_order_type,
                "Bow shot driver detached after sequence advanced past bow orders"
            );
            entity.actor_data_mut().unwrap().active_shot.clear();
            continue;
        }

        let mut direction = direction;
        let mut frame_progression = crate::sprite::FrameProgression::Default;
        if is_shoot_order(current_order_type) {
            // Original samples the live target only when the shooting order
            // initializes. Translation and bow-preparation transitions retain
            // the actor's old facing, and later target movement does not
            // rewrite this shot's goal.
            if order_initialising {
                let target_id = shot.target.unwrap_or_else(|| {
                    panic!("active bow shot for {shooter_id:?} has no target at initialization")
                });
                // The soldier override for a downward leaning-out shot uses
                // GetPositionMap on both participants. Ordinary and high bow
                // shots remain in the Human arm, which uses PositionGround.
                let leaning_out = current_order_type == OrderType::ShootingWithBowLeaningOut;
                let target_pos = (if leaning_out {
                    target_map_positions.get(target_id)
                } else {
                    target_ground_positions.get(target_id)
                })
                .and_then(|position| *position)
                .unwrap_or_else(|| {
                    panic!(
                        "active bow shot for {shooter_id:?} lost target {target_id:?} at initialization"
                    )
                });
                let shooter_pos = if leaning_out {
                    entity.element_data().position_map()
                } else {
                    let position = entity.element_data().position();
                    MapPoint::new(position.x, position.y)
                };
                let dx = target_pos.x - shooter_pos.x;
                let dy = target_pos.y - shooter_pos.y;
                entity.element_data_mut().set_direction_goal(
                    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
                );
            }
            if entity.element_data_mut().sprite.position_iface.turn() {
                frame_progression = crate::sprite::FrameProgression::FrozenFirstFrame;
            }
            direction = entity.element_data().direction();
        }
        let Ok(dir_u16) = u16::try_from(direction) else {
            tracing::warn!(
                shooter = shooter_id.index(),
                direction,
                "Bow shot tick skipped: invalid shooter direction"
            );
            continue;
        };

        // Drive the current animation through the sprite.
        //
        // Bow shots have a different headless contract than every
        // other animation site: the test in
        // `tick_bow_shots_fires_arrow_and_returns_to_aiming` (and any
        // future headless flow) expects each transition order to
        // resolve immediately when no real sprite is bound, instead of
        // staying in-progress forever.  `Sprite::perform_action`'s
        // generic empty-conversion fallback returns `InProgress`
        // (the right default for animation-tick callers, who can't
        // mark an order Impossible while scripts are still loading);
        // here we explicitly opt out and synthesize `Done`.
        let elem = entity.element_data_mut();
        let motion = if sprite_frozen {
            SpriteMotionState::InProgress
        } else if elem.sprite.scripts.is_empty() {
            if frame_progression == crate::sprite::FrameProgression::FrozenFirstFrame {
                SpriteMotionState::InProgress
            } else {
                SpriteMotionState::Done
            }
        } else {
            elem.sprite.perform_action(
                sim,
                current_order_id,
                current_order_type,
                dir_u16,
                frame_progression,
                false,
            )
        };

        let transition_start =
            is_bow_transition_order(current_order_type) && motion == SpriteMotionState::Start;
        if !transition_start
            && !matches!(
                motion,
                SpriteMotionState::Done
                    | SpriteMotionState::Terminated
                    | SpriteMotionState::Aborted
            )
        {
            continue;
        }

        // Animation for current order reached a state with bow side
        // effects or queue-completion behavior.

        if is_bow_transition_order(current_order_type) {
            // Update action state on the action-done pulse, matching the
            // C++ execute arm.  Keep the visual transition order alive until
            // Terminated so it does not collapse to a one-frame animation.
            apply_bow_transition_state_side_effect(entity, current_order_type, motion);
            if entity.is_pc()
                && !script_driven
                && motion == SpriteMotionState::Start
                && matches!(
                    current_order_type,
                    OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
                )
            {
                // RHElementActorHuman::Execute forwards this immediately
                // after setting AimingWithBow. Active-shot bow orders bypass
                // the generic actor-animation owner, so publish the same
                // callback from this mutually exclusive owner seam.
                events.pc_equip_actions.push(shooter_id);
            }
            let actor = entity.actor_data_mut().unwrap();
            if matches!(
                motion,
                SpriteMotionState::Terminated | SpriteMotionState::Aborted
            ) {
                let (remaining, bow_remaining, installed_order) = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    let installed_order =
                        elem.current_order()
                            .map(|order| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            });
                    (
                        elem.orders.is_empty(),
                        has_active_bow_order(elem),
                        installed_order,
                    )
                } else {
                    (true, false, None)
                };
                // Actor::Hourglass routes a TERMINATED bow Execute through
                // DoNextOrder, whose Proceed() result immediately replaces
                // mpOrder.  This specialized driver pops the same queue
                // directly, so publish that exact successor (or NULL) here.
                actor.installed_order = installed_order;
                if remaining || !bow_remaining {
                    actor.active_shot.clear();
                }
                if remaining {
                    events.completed.push((shot_seq_id, shot_elem_idx));
                }
            }
            continue;
        }

        let actor = entity.actor_data_mut().unwrap();
        if is_shoot_order(current_order_type) {
            if matches!(
                motion,
                SpriteMotionState::Terminated | SpriteMotionState::Aborted
            ) {
                let (remaining, bow_remaining, installed_order) = if let Some(elem) =
                    sequence_manager.get_element_mut(shot_seq_id, shot_elem_idx)
                {
                    elem.orders.pop_front();
                    let installed_order =
                        elem.current_order()
                            .map(|order| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            });
                    (
                        elem.orders.is_empty(),
                        has_active_bow_order(elem),
                        installed_order,
                    )
                } else {
                    (true, false, None)
                };
                // See the transition branch above: direct retirement must
                // mirror DoNextOrder's synchronous mpOrder publication.
                actor.installed_order = installed_order;
                if remaining || !bow_remaining {
                    actor.active_shot.clear();
                }
                if remaining {
                    events.completed.push((shot_seq_id, shot_elem_idx));
                }
                continue;
            }

            if actor.active_shot.released {
                continue;
            }
            actor.active_shot.released = true;

            // Shoot action-done pulse — arrow is released, but the
            // animation continues until Terminated.
            let Some(shot_mode) = shot.shoot_mode else {
                panic!(
                    "active bow shot missing resolved shoot mode at release: shooter={} seq_id={shot_seq_id:?} elem_idx={shot_elem_idx} order={current_order_type:?}",
                    shooter_id.index()
                );
            };
            actor.action_state = ActionState::AimingWithBow;
            if current_order_type == OrderType::ShootingWithBowLeaningOut {
                entity.element_data_mut().posture = Posture::LeaningOut;
            } else if entity.element_data().posture != Posture::AnonymousArcher {
                entity.element_data_mut().posture = Posture::Upright;
            }

            let Some(sprite_hand_point) = bow_sprite_hand_point(entity, shot_mode, direction)
            else {
                tracing::warn!(
                    shooter = shooter_id.index(),
                    ?shot_mode,
                    dir_u16,
                    "Bow release skipped: missing bow-point sprite hotspot"
                );
                continue;
            };

            let Some(target) = shot.target else {
                tracing::warn!(
                    shooter = shooter_id.index(),
                    "Bow release skipped: active shot missing target"
                );
                continue;
            };

            pending_fired.push(PendingShotTickResult {
                shooter: shooter_id,
                target,
                seq_id: shot_seq_id,
                elem_idx: shot.element_index,
                shooter_position,
                shoot_mode: shot_mode,
                shooter_direction: direction,
                sprite_hand_point,
            });
            continue;
        }

        panic!(
            "active bow shot reached unhandled active bow order: shooter={} seq_id={shot_seq_id:?} elem_idx={shot_elem_idx} order={current_order_type:?}",
            shooter_id.index()
        );
    }

    // Resolve target positions, 3D body points and forecasted movement (immutable re-borrow).
    for result in pending_fired {
        let Some(target_entity) = entities.get(result.target) else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                "Bow release skipped: target entity missing"
            );
            continue;
        };
        let target_pos = target_entity.element_data().position_map();
        let target_point = if target_entity.is_human() {
            let Some(point) = target_entity.compute_belt_point() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: human target missing belt hotspot"
                );
                continue;
            };
            point
        } else if target_entity.is_fx_target() {
            let Some(point) = target_entity.compute_target_center() else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow release skipped: FX target missing center hotspot"
                );
                continue;
            };
            point
        } else {
            tracing::warn!(
                shooter = ?result.shooter,
                target = ?result.target,
                kind = ?target_entity.kind(),
                "Bow release skipped: unsupported target kind"
            );
            continue;
        };
        let target_forecasted_movement = target_entity.position_iface().get_forecasted_movement();
        events.fired.push(ShotTickResult {
            shooter: result.shooter,
            target: result.target,
            seq_id: result.seq_id,
            elem_idx: result.elem_idx,
            shooter_position: result.shooter_position,
            target_pos,
            target_point,
            shoot_mode: result.shoot_mode,
            shooter_direction: result.shooter_direction,
            sprite_hand_point: result.sprite_hand_point,
            target_forecasted_movement,
        });
    }

    events
}

// ═══════════════════════════════════════════════════════════════════
//  Helper — launching the sequence element
// ═══════════════════════════════════════════════════════════════════

/// Build a `Command::ShootBow` sequence element on the given shooter,
/// targeting the given entity. The caller is expected to launch it via
/// `EngineInner::launch_element` so the priority is resolved eagerly.
pub fn build_shoot_bow_element(shooter: EntityId, target: EntityId) -> SequenceElement {
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(shooter));
    element.data = SequenceElementData::Interaction {
        antagonist: Some(target),
    };
    element
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests;
