//! Movement controller — advances entities along pathfinder-computed paths.
//!
//! Bridges the gap between pathfinder results (waypoint sequences) and
//! per-frame entity position updates.
//!
//! ## Flow
//!
//! 1. Pathfinder returns `Vec<MapPoint>` waypoints, stored on `ActorData`.
//! 2. Each frame, [`tick_movement`] advances the entity's position toward the
//!    current waypoint using `PositionInterface`.
//! 3. When a waypoint is reached, the index advances to the next one.
//! 4. When all waypoints are consumed, movement is complete.
//!
//! ## Speed model
//!
//! Movement orders consume per-frame sprite distances where a concrete
//! animation is available. Coarse pathing helpers still provide fixed
//! defaults for action-state-only callers.

use crate::coordinates::{MapPoint, MapVec};
use crate::element::{ActionState, EntityId};
use crate::fast_find_grid::FastFindGrid;
use crate::order::{Order, OrderType};
use crate::position_interface::{PositionInterface, TargetInfo};
use crate::sequence::{SequenceElement, SequenceId};
use crate::weapons::ShootMode;

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

/// Default movement speed for walking (map units per frame).
///
/// Speed is normally derived from per-animation-frame distance data on the
/// driving sprite (varies by sprite). The sprite-driven path is wired in
/// `engine/movement.rs tick_move` via `sprite.current_frame_distance()`;
/// this constant is only used as a fallback for the speed-lookup helpers
/// below (`speed_for_action_state` / `speed_for_order_type`) and for
/// entities that lack a sprite. Those helpers should eventually consult
/// the driving sprite rather than returning a hard-coded 1.0.
pub const DEFAULT_WALK_SPEED: f32 = 1.0;

/// Default movement speed for running (map units per frame).
pub const DEFAULT_RUN_SPEED: f32 = 6.0;

// ═══════════════════════════════════════════════════════════════════
//  Movement result
// ═══════════════════════════════════════════════════════════════════

/// Result of a single per-frame movement tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementResult {
    /// Entity is still moving toward the current waypoint.
    Moving,
    /// Reached the current waypoint and advanced to the next one.
    WaypointReached,
    /// All waypoints consumed — movement sequence is complete.
    Arrived,
    /// No active movement (no waypoints remaining).
    Idle,
}

// ═══════════════════════════════════════════════════════════════════
//  Active movement tracking
// ═══════════════════════════════════════════════════════════════════

/// Tracks the sequence element that initiated the current movement,
/// so we can notify the sequence manager when movement completes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ActiveMovement {
    pub sequence_id: Option<SequenceId>,
    pub element_index: usize,
}

impl ActiveMovement {
    pub fn none() -> Self {
        Self {
            sequence_id: None,
            element_index: 0,
        }
    }

    pub fn new(seq_id: SequenceId, elem_idx: usize) -> Self {
        Self {
            sequence_id: Some(seq_id),
            element_index: elem_idx,
        }
    }

    pub fn is_active(&self) -> bool {
        self.sequence_id.is_some()
    }

    pub fn clear(&mut self) {
        self.sequence_id = None;
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Active shot tracking (bow / thrown items)
// ═══════════════════════════════════════════════════════════════════

/// Tracks an in-progress ranged action (currently: bow shot) on an actor.
///
/// When a [`Command::ShootBow`][crate::element::Command::ShootBow] sequence
/// element is dispatched to an actor, the engine sets `ActiveShot` with the
/// target entity and the sequence element that initiated the shot. The
/// per-frame animation tick then drives the `SHOOTING_WITH_BOW` animation;
/// when it reaches the `MotionState::Done` frame, the engine spawns an
/// arrow projectile aimed at the target and notifies the sequence manager
/// that the element is terminated.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ActiveShot {
    pub sequence_id: Option<SequenceId>,
    pub element_index: usize,
    /// Target entity the shooter is aiming at.
    pub target: Option<EntityId>,
    /// Order ID used by the sprite state machine to detect "new order"
    /// transitions inside `Sprite::perform_action`.  `None` while the
    /// shot is inactive / cleared.
    pub order_id: Option<std::num::NonZeroU32>,
    /// Whether the arrow has already been released for the current
    /// shoot animation.  C++ fires on the sprite's action-done pulse,
    /// then lets the animation continue to termination.
    pub released: bool,
    /// Shoot mode resolved when the `ShootBow` command was translated.
    /// C++ keeps this as the selected shoot animation/order instead of
    /// forcing the actor's action state ahead of queued aim transitions.
    pub shoot_mode: Option<ShootMode>,
}

impl ActiveShot {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.sequence_id.is_some() && self.target.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Sweep strike state
// ═══════════════════════════════════════════════════════════════════

/// Per-frame sweep strike state for lateral/circle sword strikes.
///
/// Tracks the angular sweep: each frame the current angle advances by
/// `rotation_per_frame`, and pending victims whose direction from the
/// attacker falls within the swept arc receive damage.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct SweepState {
    /// Entities not yet hit by the sweep.
    pub pending_victims: Vec<crate::element::EntityId>,
    /// Initial sweep angle (radians).
    pub initial_angle: f32,
    /// Current sweep angle (radians) — advances each frame.
    pub current_angle: f32,
    /// Final sweep angle (radians).
    pub final_angle: f32,
    /// How much to rotate per frame (radians), signed by direction.
    pub rotation_per_frame: f32,
    /// Strike direction (determines sweep direction).
    pub direction: crate::profiles::WeaponThrustDirection,
    /// The strike type for damage application.
    pub strike: crate::weapons::SwordStrike,
    /// Attacker's weapon profile index for damage calculation.
    pub attacker_profile_idx: Option<u32>,
    /// The thrust kind — TrueCircle/FalseCircle need extended duration
    /// and per-frame attacker rotation.
    pub strike_kind: crate::profiles::WeaponThrustKind,
}

// ═══════════════════════════════════════════════════════════════════
//  Active ability tracking
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
}

impl AbilityKind {
    pub const ALL: [Self; 18] = [
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
    ];
}

/// Tracks an in-progress ability animation on an actor.
///
/// Similar to [`ActiveShot`] for bow shots. Set by `abilities::begin_*`
/// functions and consumed by owner-local `abilities::tick_ability`.
#[derive(
    Debug, Clone, Default, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ActiveAbility {
    /// Which ability is playing, or `None` if idle.
    pub kind: Option<AbilityKind>,
    /// Sequence element that initiated this ability.
    pub sequence_id: Option<SequenceId>,
    pub element_index: usize,
    /// Target entity (antagonist) for the ability, if any.
    pub target: Option<EntityId>,
    /// Order ID for the sprite animation state machine.  `None` while
    /// the ability slot is idle.
    pub order_id: Option<std::num::NonZeroU32>,
    /// Whether the legacy `RHMOTION_DONE` side effect has fired.  The
    /// selected order remains owned by the actor until `TERMINATED`.
    pub done_effect_applied: bool,
    /// Whether a Strangle order has crossed its first owner-Execute
    /// initialization boundary. Ignored by every other ability kind.
    pub strangle_initialized: bool,
}

impl ActiveAbility {
    pub fn is_active(&self) -> bool {
        self.kind.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Speed helpers
// ═══════════════════════════════════════════════════════════════════

/// Return the default movement speed for a given action state.
///
/// Pathing defaults still use coarse action states. Sprite-authored
/// per-animation distances are consumed by [`crate::sprite::Sprite`] while
/// executing concrete animation orders.
pub fn speed_for_action_state(state: ActionState) -> f32 {
    match state {
        ActionState::Moving => DEFAULT_WALK_SPEED,
        ActionState::MovingFast => DEFAULT_RUN_SPEED,
        _ => {
            tracing::warn!(
                ?state,
                "movement speed fallback returned 0.0 for action-state-only lookup; \
                 legacy implementation movement normally derives distance from the concrete sprite animation"
            );
            0.0
        }
    }
}

/// Return the default movement speed for an order type.
pub fn speed_for_order_type(order: OrderType) -> f32 {
    match order {
        OrderType::RunningUpright | OrderType::RunningStairs => DEFAULT_RUN_SPEED,
        OrderType::WalkingUpright
        | OrderType::WalkingStairs
        | OrderType::WalkingCrouched
        | OrderType::WalkingAlerted
        | OrderType::WalkingSword
        | OrderType::WalkingShield
        | OrderType::WalkingWithCorpse
        | OrderType::WalkingCarryingOnShoulders => DEFAULT_WALK_SPEED,
        _ => {
            tracing::warn!(
                ?order,
                speed = DEFAULT_WALK_SPEED,
                "movement speed fallback used hard-coded walk speed for order; \
                 legacy implementation movement normally derives distance from the concrete sprite animation"
            );
            DEFAULT_WALK_SPEED
        }
    }
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
            // `RHEngine::ProcessPathRequests` reuses the movement element's
            // last pre-path order: `NewID()`, change its action/destination,
            // and leave every preceding order in place. In particular, a
            // loaded MOVE_WAITING can still own a start transition followed
            // by a freezing order. Discarding that prefix makes the actor
            // begin moving one frame earlier than the saved Original.
            //
            // Preserve the old order's otherwise-authoritative fields just
            // like the in-place C++ mutation, while retaining Rust's typed
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

// ═══════════════════════════════════════════════════════════════════
//  Per-frame movement tick
// ═══════════════════════════════════════════════════════════════════

/// Advance an entity's position along its path waypoints for one frame.
///
/// 1. Set goal from current waypoint
/// 2. Compute normalized direction increment toward goal
/// 3. Turn entity toward movement direction
/// 4. Advance position by `distance` along the increment
/// 5. Check if goal is reached; if so, advance to next waypoint
///
/// # Arguments
///
/// * `pos` — The entity's position interface (mutable).
/// * `waypoints` — The path waypoints (from `ActorData::path_waypoints`).
/// * `waypoint_index` — Current index into `waypoints` (from `ActorData::path_waypoint_index`).
/// * `distance` — How far to move this frame (from animation speed × speed_factor).
/// * `grid` — Fast-find grid used for thick reachability checks.
/// * `target` — Target-element radius + horse flag, read live by the caller
///   (see [`TargetInfo`]).  `None` when there's no target element.
///
/// Returns a [`MovementResult`] indicating what happened.
pub fn tick_movement(
    pos: &mut PositionInterface,
    waypoints: &[MapPoint],
    waypoint_index: &mut usize,
    distance: f32,
    grid: &FastFindGrid,
    target: Option<TargetInfo>,
) -> MovementResult {
    // Get the current waypoint
    let goal = match waypoints.get(*waypoint_index) {
        Some(&wp) => wp,
        None => return MovementResult::Idle,
    };

    // Snapshot position for this frame (old_position = position)
    pos.new_move();

    // Set goal from current waypoint
    pos.set_map_goal(goal);

    // Intermediate waypoints use tolerance 0; final waypoint also uses 0
    // here because this helper takes a flat waypoint list rather than
    // per-order tolerances. Production movement runs through
    // `engine/movement.rs tick_entity_movement`, which reads each
    // `Order::tolerance` directly; this `tick_movement` is retained
    // for tests and for callers that don't have an `Order` chain.
    pos.set_tolerance(0.0, false);

    // Hint the next waypoint for anti-collision lookahead
    if let Some(&next_wp) = waypoints.get(*waypoint_index + 1) {
        pos.set_next_map_goal(next_wp);
    } else {
        pos.set_goal_next_valid(false);
    }

    // Compute increment (normalized direction toward goal) and set
    // the entity's facing direction from the movement vector.
    pos.reset_increment_computed();
    pos.compute_increment_all(true);

    // Turn one step toward the target direction.
    // The sprite-driven distance path in `engine/movement.rs` multiplies
    // distance by 0.6 while the entity is still turning toward its goal
    // direction.  This helper operates on a bare `PositionInterface`
    // without the sprite path, so the turn-slowdown factor is not
    // applied here; callers that need authentic turn-slowdown should
    // drive movement through the sprite pipeline.
    pos.turn();

    // Advance map position by increment × distance
    if distance > 0.0 {
        pos.update_position_map_scaled(distance);
    }

    // Check if we reached the current waypoint
    if pos.is_goal_reached(grid, target) {
        // Snap to exact goal when not deviated
        if !pos.is_deviated() {
            pos.set_map_position(goal);
        }

        // Zero out the movement increment
        pos.set_map_increment(MapVec::ZERO);

        // Advance to next waypoint
        *waypoint_index += 1;
        if *waypoint_index < waypoints.len() {
            MovementResult::WaypointReached
        } else {
            MovementResult::Arrived
        }
    } else {
        MovementResult::Moving
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;

    /// Helper: create a PositionInterface at a given map position.
    fn make_pos(x: f32, y: f32) -> PositionInterface {
        let mut pos = PositionInterface::new();
        pos.set_map_position(crate::coordinates::MapPoint { x, y });
        pos
    }

    #[test]
    fn tick_idle_when_no_waypoints() {
        let mut pos = make_pos(0.0, 0.0);
        let waypoints: Vec<MapPoint> = vec![];
        let mut idx = 0;
        let grid = FastFindGrid::new();
        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Idle);
    }

    #[test]
    fn tick_idle_when_index_past_end() {
        let mut pos = make_pos(0.0, 0.0);
        let waypoints = vec![map_pt(10.0, 0.0)];
        let mut idx = 1; // past end
        let grid = FastFindGrid::new();
        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Idle);
    }

    #[test]
    fn tick_moves_toward_waypoint() {
        let mut pos = make_pos(0.0, 0.0);
        let waypoints = vec![map_pt(100.0, 0.0)];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Moving);

        // Position should have advanced toward (100, 0)
        let map = pos.map_position();
        assert!(map.x > 0.0, "should have moved in +x direction");
        assert!(map.x.abs() < 2.0, "should have moved ~1 unit");
    }

    #[test]
    fn tick_reaches_nearby_waypoint() {
        // Start very close to waypoint — should arrive in one tick
        let mut pos = make_pos(99.5, 0.0);
        let waypoints = vec![map_pt(100.0, 0.0)];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Arrived);
        assert_eq!(idx, 1);

        // Should have snapped to goal
        let map = pos.map_position();
        assert!((map.x - 100.0).abs() < 0.01);
    }

    #[test]
    fn tick_advances_through_multiple_waypoints() {
        // Start right on top of the first waypoint — should reach it immediately
        let mut pos = make_pos(10.0, 0.0);
        let waypoints = vec![
            map_pt(10.0, 0.0),  // Already there
            map_pt(100.0, 0.0), // Far away
        ];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        // First tick: should reach waypoint 0 and advance
        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::WaypointReached);
        assert_eq!(idx, 1);

        // Second tick: should be moving toward waypoint 1
        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Moving);
    }

    #[test]
    fn tick_full_path_traversal() {
        let mut pos = make_pos(0.0, 0.0);
        let waypoints = vec![map_pt(2.0, 0.0), map_pt(4.0, 0.0), map_pt(6.0, 0.0)];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        // Run enough ticks to traverse all waypoints
        let mut arrived = false;
        for _ in 0..100 {
            let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
            if result == MovementResult::Arrived {
                arrived = true;
                break;
            }
        }

        assert!(arrived, "should have arrived at destination");
        assert_eq!(idx, 3);

        // Should be at/near the final waypoint
        let map = pos.map_position();
        assert!(
            (map.x - 6.0).abs() < 1.0,
            "should be near final waypoint, got {}",
            map.x
        );
    }

    #[test]
    fn tick_diagonal_movement() {
        let mut pos = make_pos(0.0, 0.0);
        let waypoints = vec![map_pt(100.0, 100.0)];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        let result = tick_movement(&mut pos, &waypoints, &mut idx, 1.0, &grid, None);
        assert_eq!(result, MovementResult::Moving);

        let map = pos.map_position();
        // Should move diagonally — both x and y should increase
        assert!(map.x > 0.0);
        assert!(map.y > 0.0);
        // Diagonal unit vector is (0.707, 0.707), so each component ≈ 0.707
        assert!(
            (map.x - map.y).abs() < 0.01,
            "diagonal movement should be symmetric"
        );
    }

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

    #[test]
    fn speed_for_action_state_values() {
        assert_eq!(
            speed_for_action_state(ActionState::Moving),
            DEFAULT_WALK_SPEED
        );
        assert_eq!(
            speed_for_action_state(ActionState::MovingFast),
            DEFAULT_RUN_SPEED
        );
        assert_eq!(speed_for_action_state(ActionState::Waiting), 0.0);
    }

    #[test]
    fn speed_for_order_type_values() {
        assert_eq!(
            speed_for_order_type(OrderType::WalkingUpright),
            DEFAULT_WALK_SPEED
        );
        assert_eq!(
            speed_for_order_type(OrderType::RunningUpright),
            DEFAULT_RUN_SPEED
        );
        assert_eq!(
            speed_for_order_type(OrderType::WalkingCrouched),
            DEFAULT_WALK_SPEED
        );
    }

    #[test]
    fn active_movement_tracking() {
        let mut am = ActiveMovement::none();
        assert!(!am.is_active());

        am = ActiveMovement::new(SequenceId(42), 3);
        assert!(am.is_active());
        assert_eq!(am.sequence_id, Some(SequenceId(42)));
        assert_eq!(am.element_index, 3);

        am.clear();
        assert!(!am.is_active());
    }

    #[test]
    fn zero_distance_does_not_move() {
        let mut pos = make_pos(50.0, 50.0);
        let waypoints = vec![map_pt(100.0, 100.0)];
        let mut idx = 0;
        let grid = FastFindGrid::new();

        let result = tick_movement(&mut pos, &waypoints, &mut idx, 0.0, &grid, None);
        assert_eq!(result, MovementResult::Moving);

        // Position should not have changed
        let map = pos.map_position();
        assert!((map.x - 50.0).abs() < 0.01);
        assert!((map.y - 50.0).abs() < 0.01);
    }
}
