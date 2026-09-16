//! Bow-shot execution — dispatch logic for `Command::ShootBow`.
//!
//! Implements the end-to-end flow for firing an arrow at a target:
//!
//! 1. [`begin_bow_shot`] is called by the engine when a
//!    `Command::ShootBow` sequence element is dispatched to a shooter.
//!    It sets the shooter into the appropriate aiming action state,
//!    pushes aim-transition and shoot orders onto the order queue,
//!    on the sequence element.
//!
//! 2. The engine executes each actor's selected bow order directly,
//!    including equip callbacks, arrow release, and order completion.
//!    At release it looks up the shooter's bow profile, rolls the hit chance,
//!    computes a ballistic trajectory via [`compute_initial_throw_velocity`]
//!    and [`compute_trajectory_ballistic`], and spawns the arrow via
//!    [`spawn_arrow`].
//!
//! 3. Each projectile advances at its position in the entity update order.
//!    Collision queries read live entities, and impact callbacks immediately
//!    launch damage or target activation before the next entity updates.
//!
//! ## UI action-slot refresh
//!
//! When ammo reaches 0, the ammo decrement path
//! (`engine/archery.rs::decrement_bow_ammo`) calls
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
use crate::entities::Entities;
use crate::order::{Order, OrderType};
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};
use crate::profiles::{Action, ProfileManager};
use crate::sequence::{SequenceElement, SequenceElementData, SequenceId, SequenceManager};
use crate::sprite::MotionState as SpriteMotionState;
use crate::weapons::ShootMode;

mod collision;
mod projectile;
mod trajectory;

#[cfg(test)]
use collision::shield_params_for_soldier;
pub use collision::{ShieldParams, compute_shield_obstacle, shield_params_for_pc};
pub(crate) use collision::{
    refresh_retained_shield_obstacle, shield_obstacle_from_serialized_state,
};
pub use projectile::{
    APEX_BEGGAR_COIN, APEX_COIN, BOUNCE_COIN, COIN_SCATTER_ATTEMPTS, COIN_SCATTER_MIN,
    COIN_SCATTER_RANGE, MASS_COIN, NUMBER_OF_COINS_IN_PURSE, NUMBER_OF_WASPS, SpawnArrowParams,
    apply_arrow_hit, spawn_apple, spawn_arrow, spawn_coin, spawn_net, spawn_purse, spawn_stone,
    spawn_wasp, spawn_wasp_nest,
};
pub(crate) use projectile::{
    make_arrow_falling_down, projectile_human_victim, projectile_shield_holder,
    projectile_target_victim, refresh_arrow_after_previous_hourglass,
};
use trajectory::compute_trajectory_ballistic_bounce;
pub use trajectory::{
    TrajectoryObstacleCheck, apply_projectile_landing_resolution, bind_trajectory_obstacle,
    compute_bow_point, compute_initial_throw_velocity, compute_shot_velocity_params,
    compute_trajectory_ballistic, compute_trajectory_ballistic_with_terminal_impact,
    compute_trajectory_ballistic_with_terminal_obstacle, roll_hit_and_compute_bias,
    terminal_obstacle_plane, will_hit_target,
};

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

pub(crate) fn set_projectile_animation(proj: &mut ElementProjectile, animation: Animation) {
    proj.object.animation = animation;
    if proj.element.sprite.current_conversion().is_empty() {
        return;
    }
    assert!(
        proj.element.sprite.has_animation(animation),
        "projectile {:?} is missing required animation {animation:?}",
        proj.object.object_type
    );
    // The original game's apple and stone collision handling uses the
    // animation initialization without an explicit direction, which defaults to 0.
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

/// The original game's bow-point calculation selects these non-anonymous
/// animation ids for hotspot lookup even when the active shoot animation is
/// an anonymous archer variant.
pub(crate) fn bow_point_order_type_for_mode(mode: ShootMode) -> OrderType {
    match mode {
        ShootMode::Normal => OrderType::ShootingWithBow,
        ShootMode::Long => OrderType::ShootingWithBowUp,
        ShootMode::Down => OrderType::ShootingWithBowLeaningOut,
    }
}

/// Absolute projected bow hotspot used by the original game: sprite position
/// plus the animation and direction hotspot.
pub(crate) fn bow_sprite_hand_point(
    entity: &Entity,
    mode: ShootMode,
    direction: i16,
) -> Option<MapPoint> {
    let dir = u16::try_from(direction).ok()?;
    let sprite_pos = entity.gameplay_sprite_position();
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

pub(crate) fn is_shoot_order(ot: OrderType) -> bool {
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
pub(crate) fn is_bow_transition_order(ot: OrderType) -> bool {
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

pub(crate) fn apply_bow_transition_state_side_effect(
    engine: &mut crate::engine::EngineInner,
    entity_id: EntityId,
    order_type: OrderType,
    motion: SpriteMotionState,
) {
    let action_state = match order_type {
        OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
            if motion == SpriteMotionState::Start =>
        {
            if engine
                .expect_entity(entity_id, "bow posture owner")
                .posture()
                != Posture::AnonymousArcher
            {
                engine.publish_entity_order_posture(entity_id, Posture::Upright);
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
            engine.publish_entity_order_posture(entity_id, Posture::LeaningOut);
            Some(ActionState::AimingWithBowDown)
        }
        OrderType::TransitionRaisingBowLeaningOut
            if matches!(
                motion,
                SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            engine.publish_entity_order_posture(entity_id, Posture::Upright);
            Some(ActionState::AimingWithBow)
        }
        OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous
            if matches!(
                motion,
                SpriteMotionState::Start | SpriteMotionState::Done | SpriteMotionState::Terminated
            ) =>
        {
            if engine
                .expect_entity(entity_id, "bow posture owner")
                .posture()
                != Posture::AnonymousArcher
            {
                engine.publish_entity_order_posture(entity_id, Posture::Upright);
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
        && let Some(actor) = engine
            .expect_entity_mut(entity_id, "bow state owner")
            .actor_data_mut()
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

pub(crate) fn bow_target_ground_position(entity: &Entity) -> MapPoint {
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
pub fn begin_bow_shot(
    entities: &Entities,
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
    // Original-game bow-shot eligibility does not test active state:
    // Original-game element removal deliberately retains removed objects because
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
    let (shooter_posture, current_state) = match entities.get(shooter_id) {
        Some(e) if e.is_human() && !e.is_dead() => {
            let posture = e.element_data().posture();
            let Some(actor) = e.actor_data() else {
                tracing::warn!(
                    shooter = ?shooter_id,
                    "Begin bow shot rejected: human shooter missing actor data"
                );
                return BeginShotResult::Impossible;
            };
            (posture, actor.action_state)
        }
        _ => return BeginShotResult::Impossible,
    };

    // The original game's shoot-bow translation chooses
    // raise/lower setup from the sequence element's
    // post-transition action state, not from the actor's live state. That
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

    // Push aim-transition orders if needed.  Orders live on the owning
    // `SequenceElement.orders` — when the element is cancelled, its
    // orders go with it.
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
