//! Entity hierarchy — the base class system for all game entities.
//!
//! Animal actors are *not* implemented — no shipped Robin Hood level
//! instantiates any animal, so the whole subsystem was ripped out.  The
//! BETE/MEOW mission-file chunk is still parsed by
//! `level_data::read_animals`, which panics if it ever encounters
//! a non-empty animal count.
//!
//! ## Design
//!
//! The hierarchy is conceptually:
//! ```text
//! Element (base)
//! ├── Actor → Human → PC | NPC (Soldier, Civilian)
//! ├── Fx → Target
//! └── Object → Bonus | Projectile → Net
//! ```
//!
//! Mission chariot masters live in `WorldState::mobile_elements`; their
//! masked child sprites use `Entity::Fx` slots tagged by
//! `FxData::mobile_index`.
//!
//! This Rust port uses:
//! - **Composition**: Each hierarchy level has its own `*Data` struct.
//!   Concrete entity types compose these structs flat.
//! - **Traits**: Shared behavior interfaces via trait inheritance
//!   (`Element`, `Actor`, `Human`).
//! - **Enum dispatch**: [`Entity`] enum holds any concrete entity type for
//!   exhaustive pattern matching without `dyn`.
//! - **[`EntityId`]**: Cross-entity references use IDs, not pointers.

use std::collections::VecDeque;
use std::fmt;
use std::hash::Hasher;

use serde::{Deserialize, Serialize};

use crate::ai::{AiController, AiState as AiTopState, Substate as AiSubstate};
use crate::ai_enemy::EnemyAi;
use crate::ai_friendly::FriendlyAi;
use crate::coordinates::{
    GroundPoint, MapBBox, MapPoint, MapVec, SpriteTopLeft, WorldPoint3D, WorldVec3D,
};
use crate::fast_find_grid::GRID_CELL_SIZE;
use crate::human_control::{
    CombatStance, CommandInterface, DecisionPolicy, HumanArchetype, HumanControlProfile,
    MissionRole,
};
use crate::jump_line::JumpLineIndex;
use crate::movement::{ActiveMovement, ActiveShot};
use crate::order::OrderType;
use crate::position_interface::{PositionInterface, SectorHandle};
use crate::profiles::{
    Action, CharacterProfile, CharacterProfileIdx, CivilianProfileIdx, SoldierProfileIdx,
};
use crate::sprite::Sprite;

mod entity;
mod persisted;
mod traits;
pub(crate) use persisted::*;
pub use traits::{Actor, Element, Human};

/// Re-export: `OrderType` is the canonical animation-type enum.
pub type Animation = OrderType;

// ═══════════════════════════════════════════════════════════════════
//  Entity identity
// ═══════════════════════════════════════════════════════════════════

pub use crate::element_kinds::*;
pub use crate::entity_id::{
    ActorId, BonusId, CivilianId, EntityId, EntityIdKind, FxId, HumanId, NetId, NpcId, ObjectId,
    PcId, ProjectileId, ScrollId, SoldierId, TargetId,
};
// ═══════════════════════════════════════════════════════════════════
//  Data structs — one per hierarchy level
// ═══════════════════════════════════════════════════════════════════

/// Base data shared by **all** entities.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementData {
    pub kind: ElementKind,

    // Identity
    pub blipped: bool,
    pub class_id: u16,
    pub active: bool,
    /// Whether the actor is hidden inside a building.
    /// Set when entering a building sector; separate from `active` so the
    /// entity still participates in game logic.
    pub hidden_in_building: bool,

    // Sprite surface
    pub sprite_id: u32,
    pub select_id: u32,

    /// Original-game queued map-space teleport. The actor update
    /// consumes this before selecting/executing the current order.
    pub position_map_delayed: bool,
    pub delayed_map_position: MapPoint,

    /// Original-game queued world-space teleport. When both queues
    /// are armed, the actor update consumes the map-space queue first and
    /// leaves this one for the following frame.
    pub position_delayed: bool,
    pub delayed_position: WorldPoint3D,

    /// "Teleported away" by script.
    pub in_honolulu: bool,

    pub index_in_elements_list: u16,
    pub custom_minimap_dot: u16,

    // Outline colours
    pub outline_colors: [u16; OutlineColorName::COUNT],
    pub current_outline: OutlineColorName,
    pub outline_width: u16,

    pub unreachable: bool,

    /// Current posture. Runtime writes must go through
    /// [`ElementData::set_posture`] or an entity-level helper so the
    /// corpse-transition guard stays centralized.
    posture: Posture,

    // -- Cross-module references --
    /// The entity's sprite animation/rendering state + embedded
    /// `PositionInterface` (position/direction/layer/sector/material/...).
    pub sprite: Sprite,

    /// Cached grid cell coordinates `(cx, cy)` for fast_find_grid spatial
    /// queries.  Updated whenever the entity moves.
    pub grid_cell: Option<(u16, u16)>,
}

impl Default for ElementData {
    fn default() -> Self {
        Self {
            kind: ElementKind::Fx,
            blipped: false,
            class_id: 0,
            active: true,
            hidden_in_building: false,
            sprite_id: 0,
            select_id: 0,
            position_map_delayed: false,
            delayed_map_position: MapPoint::ZERO,
            position_delayed: false,
            delayed_position: WorldPoint3D::ZERO,
            in_honolulu: false,
            index_in_elements_list: 0,
            // Default to `CUSTOM_DOT_NOT_CUSTOMIZED` (=1). Zero would
            // mean Invisible, silently hiding every entity on the
            // minimap.
            custom_minimap_dot: 1,
            outline_colors: [0; OutlineColorName::COUNT],
            current_outline: OutlineColorName::Default,
            outline_width: 2,
            unreachable: false,
            posture: Posture::Undefined,
            sprite: Sprite::default(),
            grid_cell: None,
        }
    }
}

impl ElementData {
    // -- PositionInterface forwarding accessors --
    //
    // Position/direction/layer/sector/material/obstacle live on the
    // embedded `PositionInterface` (inside `sprite`). These forwarders
    // exist so callers can keep using `elem.direction()` / `elem.position()`
    // etc. rather than threading `sprite.position_iface` through every
    // access.

    #[inline]
    #[must_use = "method returns a value by value; assigning to its fields is a silent no-op (e.g. `elem.direction()` then `+= 1` modifies a temporary). Use the `set_direction_*` setters instead."]
    pub fn direction(&self) -> i16 {
        self.sprite.position_iface.get_direction().into()
    }
    #[inline]
    pub fn set_direction_instantly(&mut self, d: i16) {
        self.sprite
            .position_iface
            .set_direction_instantly(crate::position_interface::Direction::from_raw(d as i32));
    }
    #[inline]
    pub fn set_direction_goal(&mut self, d: i16) {
        self.sprite
            .position_iface
            .set_direction(crate::position_interface::Direction::from_raw(d as i32));
    }

    #[inline]
    #[must_use = "method returns WorldPoint3D by value; `elem.position().x = v` (or `+= v`) silently modifies a temporary. Use `set_position` to mutate."]
    pub fn position(&self) -> WorldPoint3D {
        let p = self.sprite.position_iface.get_position();
        WorldPoint3D {
            x: p.x,
            y: p.y,
            z: p.z,
        }
    }
    #[inline]
    pub fn set_position(&mut self, p: WorldPoint3D) {
        self.sprite
            .position_iface
            .set_position(crate::coordinates::WorldPoint3D {
                x: p.x,
                y: p.y,
                z: p.z,
            });
    }

    #[inline]
    #[must_use = "method returns MapPoint by value; `elem.position_map().x = v` silently modifies a temporary. Use `set_position_map` to mutate."]
    pub fn position_map(&self) -> MapPoint {
        self.sprite.position_iface.map_position()
    }

    #[inline]
    pub fn set_position_map(&mut self, p: MapPoint) {
        self.sprite.position_iface.set_map_position(p);
    }

    #[inline]
    pub fn set_position_map_preserving_3d(&mut self, p: MapPoint) {
        self.sprite.position_iface.set_map_position_preserving_3d(p);
    }

    /// Complete the position-interface publication performed by Original's
    /// projectile tick: set the increment, update position, then
    /// recompute all coordinate representations.
    pub(crate) fn finish_projectile_position_update(&mut self, increment: WorldVec3D) {
        let map = self.position_map();
        let center = self.sprite.center;
        self.sprite
            .position_iface
            .set_projectile_increment(increment);
        self.sprite
            .position_iface
            .finish_flight_position_update(MapPoint::new(
                (map.x - center.x).floor(),
                (map.y - center.y).floor(),
            ));
    }

    /// Queue a map-space position change for the next actor update.
    pub fn set_position_map_delayed(&mut self, point: MapPoint) {
        self.delayed_map_position = point;
        self.position_map_delayed = true;
    }

    /// Queue a world-space position change for the next actor update.
    pub fn set_position_delayed(&mut self, point: WorldPoint3D) {
        self.delayed_position = point;
        self.position_delayed = true;
    }

    /// Apply the next queued Original position update, preserving the
    /// map-before-world priority and the inactive queue's stored point.
    ///
    /// Returns the pre/post map segment and layer for the Actor
    /// line-crossing-check continuation.
    pub(crate) fn apply_next_delayed_position(&mut self) -> Option<(MapPoint, MapPoint, u16)> {
        if !self.position_map_delayed && !self.position_delayed {
            return None;
        }

        let old_position = self.position_map();
        let layer = self.layer();
        self.sprite.position_iface.new_move();
        if self.position_map_delayed {
            self.set_position_map(self.delayed_map_position);
            self.position_map_delayed = false;
        } else {
            self.set_position(self.delayed_position);
            self.position_delayed = false;
        }
        self.update_grid_cell();
        Some((old_position, self.position_map(), layer))
    }

    #[inline]
    #[must_use]
    pub fn layer(&self) -> u16 {
        self.sprite.position_iface.get_layer().into()
    }
    #[inline]
    #[must_use]
    pub fn optional_layer(&self) -> Option<crate::position_interface::Layer> {
        self.sprite.position_iface.optional_layer()
    }
    #[inline]
    pub fn set_layer(&mut self, l: u16) {
        let layer = crate::position_interface::Layer::new(l)
            .expect("layer must be < 0xFFFF; 0xFFFF is the 'no layer' sentinel");
        self.sprite.position_iface.set_layer(layer);
    }
    #[inline]
    pub fn clear_layer(&mut self) {
        self.sprite.position_iface.clear_layer();
    }

    #[inline]
    #[must_use]
    pub fn sector(&self) -> Option<crate::position_interface::SectorHandle> {
        self.sprite.position_iface.get_sector()
    }
    #[inline]
    pub fn set_sector(&mut self, s: Option<crate::position_interface::SectorHandle>) {
        self.sprite.position_iface.set_sector(s);
    }
    #[inline]
    pub fn set_sector_topology(
        &mut self,
        sector: Option<crate::position_interface::SectorHandle>,
        sector_index: Option<crate::fast_find_grid::SectorIndex>,
    ) {
        self.sprite
            .position_iface
            .set_sector_topology(sector, sector_index);
    }

    /// Door-transit half of "is inside a building": true while the
    /// actor is in the middle of a pass-door animation, before its
    /// sector pointer has been swapped to the inside-building sector.
    #[inline]
    pub fn is_in_door_transit(&self) -> bool {
        self.sprite.position_iface.get_door().is_some()
    }

    #[inline]
    #[must_use]
    pub fn obstacle_index(&self) -> Option<crate::position_interface::ObstacleHandle> {
        self.sprite.position_iface.get_obstacle()
    }
    /// Set the obstacle the element is standing on. The caller must
    /// supply the obstacle's pre-resolved top-plane coefficients — the
    /// obstacle pointer and its top plane are paired whenever the
    /// obstacle is non-null.
    #[inline]
    pub fn set_obstacle_index(
        &mut self,
        obs: Option<crate::position_interface::ObstacleHandle>,
        plane: Option<crate::position_interface::PlaneZCoeffs>,
    ) {
        self.sprite.position_iface.set_obstacle(obs, plane);
    }

    #[inline]
    #[must_use]
    pub fn material(&self) -> GameMaterial {
        self.sprite.position_iface.get_material()
    }
    #[inline]
    pub fn set_material(&mut self, m: GameMaterial) {
        self.sprite.position_iface.set_material(m);
    }

    /// Change the posture, respecting the corpse-transition guard: a
    /// `Dead` / `DeadBack` corpse can only transition to `Carried`
    /// (pickup); any other posture write on a dead sprite is silently
    /// dropped. This is the public transition API; internal order publication
    /// and save adoption have explicitly separate semantics.
    ///
    /// The "fire intersection update on every lying↔non-lying
    /// transition" hook is implemented as a deferred per-tick drain
    /// rather than a synchronous hook on this setter: the hook needs
    /// engine access to iterate actors. See
    /// [`EngineInner::process_corpse_intersection_updates`].
    pub fn set_posture(&mut self, p: Posture) {
        if self.posture.allows_transition_to(p) {
            self.posture = p;
            self.sprite.position_iface.set_posture(p);
        }
    }

    /// Gameplay posture, read without exposing mutation authority.
    ///
    /// ```compile_fail
    /// use robin_engine::element::{ElementData, Posture};
    /// let mut element = ElementData::default();
    /// element.posture = Posture::Upright;
    /// ```
    pub fn posture(&self) -> Posture {
        self.posture
    }

    /// Publish a logical animation/order posture without rewriting the sprite
    /// position's independently retained posture. Unlike a requested posture
    /// transition, an executing order publishes this value unconditionally.
    /// The animation and damage coordinators historically publish these at
    /// different callback barriers, including frozen RunningUpright execution.
    /// TODO: unify the two posture sources only with replay/hash evidence for
    /// those barriers; synchronizing them here changes saved simulation state.
    pub(crate) fn publish_order_posture(&mut self, posture: Posture) {
        self.posture = posture;
    }

    /// Construct an element's initial gameplay posture before attaching its
    /// independently prepared sprite. This is initialization, not a runtime
    /// transition: the sprite's saved posture must not be overwritten here.
    pub fn from_initial_posture(posture: Posture) -> Self {
        Self {
            posture,
            ..Self::default()
        }
    }

    /// Restore the original save's single authoritative position/posture
    /// record without applying the live corpse-transition guard.
    pub(crate) fn restore_v48_position_and_posture(
        &mut self,
        position: crate::position_interface::PositionInterfaceV48State,
    ) {
        self.posture = position.posture;
        self.sprite
            .position_iface
            .restore_v48_serialized_state(position);
    }

    /// Recompute the cached grid cell from the current map position.
    pub fn update_grid_cell(&mut self) {
        let pm = self.position_map();
        let cx = (pm.x as i32 / GRID_CELL_SIZE) as u16;
        let cy = (pm.y as i32 / GRID_CELL_SIZE) as u16;
        self.grid_cell = Some((cx, cy));
    }

    /// Reveal a blipped entity: clear the blip flag AND flip the sprite
    /// back to its primary (normal-character) profile so it stops
    /// rendering as a dark silhouette.
    ///
    /// Clears the `blipped` flag and swaps `use_alternate_profile` back
    /// off so the renderer (which reads through `current_scripts_opt`)
    /// picks up the real character sprite instead of the blip00
    /// silhouette.
    ///
    /// After the profile swap the sprite's `current_row` / `current_frame`
    /// can land on indices that are valid for the blip00 profile but
    /// out-of-range for the revealed character (blip00 has a single
    /// idle row; the real character has many).  We reset them to 0 so
    /// the renderer always picks a valid frame; the next animation
    /// command from the AI will re-drive the row based on the actor's
    /// real action state.
    ///
    /// Safe to call on non-blipped entities: it's a no-op beyond
    /// clearing a flag that's already false.
    /// `direction` is the sprite's current facing.  Callers on an
    /// `Entity` should pass `actor_data().position_iface.get_direction()`
    /// — `element.direction` can lag pi.direction by a frame.
    pub fn reveal_blip(&mut self, direction: u16) {
        if !self.blipped {
            return;
        }
        self.blipped = false;
        let sprite = &mut self.sprite;
        // Only flip if the sprite actually loaded the alternate
        // profile — otherwise `use_alternate_profile` is false by
        // default and toggling it would select a non-existent slot.
        if sprite.alternate_scripts.is_some() && sprite.use_alternate_profile {
            // Just toggle the profile.  `switch_alternate_profile`
            // recomputes `current_row` from `last_action` + direction
            // when an animation has already played — without that, a
            // newly-revealed guard reverts to row 0 (north-facing
            // WaitingUpright) instead of keeping its authored
            // direction.
            sprite.switch_alternate_profile(direction & 15);
        }
    }

    /// Initialise outline colours based on entity kind.
    ///
    /// Per-subclass colour table:
    /// - PC → green
    /// - Soldier → red / purple (VIP)
    /// - Civilian → cyan / purple (VIP)
    /// - Object → yellow
    /// - Target → red
    ///
    /// `is_vip` selects the VIP branch for soldiers: VIP soldiers write
    /// only the Hidden/Default/Target slots with the purple
    /// `OC_NPC_VIP_*` values and leave Striking/Parrying untouched
    /// (encoding "VIPs don't strike").  Civilian VIP support is not
    /// yet wired through (`init_outline_colors` callers don't provide
    /// a civilian profile); for now non-soldier kinds ignore `is_vip`.
    pub fn init_outline_colors(&mut self, is_vip: bool) {
        use OutlineColorName as N;
        use outline_colors::*;

        match self.kind {
            ElementKind::ActorPc => {
                self.outline_colors[N::Default as usize] = pc_default();
                self.outline_colors[N::Hidden as usize] = pc_hidden();
                self.outline_colors[N::Target as usize] = pc_target();
            }
            ElementKind::ActorSoldier => {
                if is_vip {
                    // VIP branch: only Hidden / Default / Target are
                    // written; Striking / Parrying stay at their
                    // default zero values, encoding "VIPs don't
                    // strike".
                    self.outline_colors[N::Default as usize] = npc_vip_default();
                    self.outline_colors[N::Hidden as usize] = npc_vip_hidden();
                    self.outline_colors[N::Target as usize] = npc_vip_target();
                } else {
                    self.outline_colors[N::Default as usize] = npc_evil_default();
                    self.outline_colors[N::Hidden as usize] = npc_evil_hidden();
                    self.outline_colors[N::Target as usize] = npc_evil_target();
                    self.outline_colors[N::Striking as usize] = npc_evil_striking();
                    self.outline_colors[N::Parrying as usize] = npc_evil_parrying();
                }
            }
            ElementKind::ActorCivilian => {
                self.outline_colors[N::Default as usize] = npc_good_default();
                self.outline_colors[N::Hidden as usize] = npc_good_hidden();
                self.outline_colors[N::Target as usize] = npc_good_target();
            }
            ElementKind::Target => {
                // The red target colour goes in the `Default` slot,
                // not `Target` — an FX target uses the *default*
                // outline as its red highlight.
                self.outline_colors[N::Default as usize] = target_target();
            }
            _ => {
                self.outline_colors[N::Hidden as usize] = object_hidden();
                self.outline_colors[N::Target as usize] = object_target();
            }
        }
    }

    /// Get the active outline colour (RGB565).  Returns 0 if no colour is set.
    pub fn active_outline_color(&self) -> u16 {
        self.outline_colors[self.current_outline as usize]
    }
}

/// In-progress ladder / wall climb.
///
/// Set when an actor enters a wall-or-ladder lift sector (the WAIT_FREE_LIFT
/// command's success path in `tick.rs`), cleared when the actor finishes
/// crossing the door on the other side (the "Leaving a lift" branch in
/// `door_pass.rs`). Lets the push-damage path know which sector an actor
/// was climbing so `translate_ladder_wall_fall` can decrement that
/// sector's occupancy counter.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActiveLiftClimb {
    /// The lift sector number the actor is currently occupying.
    pub sector_number: u16,
    /// `true` if the actor entered at the top going down, `false` if
    /// they entered at the bottom going up. Used to flip the correct
    /// `lift_occupied_*` flag back off on exit.
    pub upwards: bool,
}

/// Active push-flight state.
///
/// When a push/circle/charge strike lands, the victim is launched along a
/// flight vector over several animation frames instead of teleporting
/// instantly.  Each frame the position is advanced by `increment`; on the
/// final frame the entity snaps to `goal`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum FlightGeometry {
    /// Movement is expressed in projected map coordinates; the current
    /// ground plane remains authoritative for elevation.
    #[default]
    GroundPlane,
    /// Takeoff preparation resolved a complete world-space endpoint, including
    /// its projection obstacle and elevation.
    World3d,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActiveFlight {
    pub geometry: FlightGeometry,
    /// Per-frame position increment (total displacement / frames). For
    /// [`FlightGeometry::World3d`], Y is the cached world-space Y increment;
    /// the projected map-space increment is `increment_y - increment_z`.
    pub increment_x: f32,
    pub increment_y: f32,
    /// Goal position to snap to on completion.
    pub goal_x: f32,
    pub goal_y: f32,
    /// Frames remaining in the flight.
    pub frames_remaining: u16,
    /// Original hitter that launched this flight, when the flight was
    /// triggered by a hit/push strike. Each frame the flyer applies a
    /// domino-effect sweep to nearby upright actors and propagates a
    /// `ReceiveHitDamage` element citing this antagonist.
    ///
    /// `None` for non-combat flights (rolling, ladder/wall fall) where
    /// the domino-effect sweep is not invoked.
    pub antagonist: Option<EntityId>,

    /// Per-frame z (elevation) increment.  Non-zero only when the goal
    /// sits on a sloped projection-area obstacle (currently only set by
    /// push flights — `apply_push_effect`); other flight setup sites
    /// (rolling, ladder-wall fall, hit fall) leave this at 0.
    pub increment_z: f32,
    /// Goal elevation to snap to on completion.  Computed from the
    /// projection-area obstacle's top plane at the chosen flight goal.
    pub goal_z: f32,
    /// Goal layer to write back to the actor on landing.
    pub goal_layer: u16,
    /// Goal sector to write back to the actor on landing.
    pub goal_sector: Option<crate::position_interface::SectorHandle>,
    /// Projection-area obstacle the actor is flying onto, if any.  The
    /// actor is considered to be on the goal obstacle for the duration
    /// of the flight; we apply the obstacle on landing alongside the
    /// goal layer/sector.  Mid-flight queries that need the plane
    /// should use the explicit `increment_z` field.
    pub obstacle: Option<crate::position_interface::ObstacleHandle>,
    /// Ladder/wall fall marker.  These flights use the constant-speed
    /// kinematics of the original ladder fall (fixed 3D step length 10,
    /// duration `0.1 * distance` ticks): the flight tick mirrors the
    /// remaining tick count into `actor.wait_time`, and landing applies
    /// the fall's concussion, lying posture, and order retirement
    /// instead of the generic combat-fall completion.
    pub ladder_fall: bool,
}

/// Active rider charge state.
///
/// The Original keeps only the candidate list between calls to
/// rider-charge execution; origin, direction, layer, and animation frame are
/// sampled live on every owner movement slot.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActiveRiderCharge {
    /// Candidate victims (entities inside the initial large hit zone).
    /// Removed as they get hit.
    pub pending_victims: Vec<EntityId>,
}

/// One step in a door-pass sub-order chain.
///
/// Built by `translate_pass_door_*`. Each door type produces a specific
/// sequence of walk/transition/trigger steps.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum DoorPassStep {
    /// Walk to destination with the given animation.
    Walk {
        destination: MapPoint,
        action: crate::order::OrderType,
        reverse: bool,
        compute_direction: bool,
        /// Optional walk-step tolerance.  Used by the ladder / wall
        /// translators so the walk-to-mid step ends early enough for
        /// the subsequent climb transition to land at the exact
        /// lift-edge pixel (e.g. `TELEPORT_LADDER = 45.0`,
        /// `TELEPORT_WALL = 60.0`, or per-animation distances via
        /// per-animation distance). Stairs / building door passes
        /// leave this at `0.0`.
        tolerance: f32,
    },
    /// Fire the PassDoor() callback — change layer/sector, building/lift callbacks.
    /// First trigger changes layer/sector; second re-enables anti-collision.
    PassingDoor,
    /// Play a transition animation in place (crouch, climb transition, turn).
    Transition {
        action: crate::order::OrderType,
        reverse: bool,
    },
    /// Fire a selection-flash hulk effect on self (and carried, if any).
    /// Inserted by the building-door translator between the walk-to-mid
    /// and `PassingDoor` steps for PCs.  The handler calls
    /// hulk startup with default animation, level 2, and the supplied tolerance.
    Select {
        /// Speed factor for the hulk fade —
        /// the normalized midpoint-to-endpoint vector scaled by 0.03.
        speed: f32,
    },
}

/// Active door-pass state on an actor.
///
/// Tracks the multi-step walk-through sequence built by the
/// `translate_pass_door_*` functions (engine/door_pass.rs).
/// The movement tick processes steps one at a time:
/// - Walk steps set waypoints on the actor path
/// - PassingDoor steps fire the layer/sector swap callback
/// - Transition steps play animations in place
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActiveDoorPass {
    /// Door index in the global door table.
    pub door_index: crate::gate::DoorIndex,
    /// Direction: true = outside→inside (direct), false = inside→outside.
    pub direct: bool,
    /// Direction stored on the owning movement element.
    ///
    /// This normally equals `direct`. A v48 save can resume after the actor
    /// has already crossed the gate, however, so Rust rebuilds the remaining
    /// physical steps from the actor's destination-side sector while the
    /// Original movement element retains its initial traversal direction.
    /// AI `Position(actor)` and route-source queries read that retained value.
    pub position_direct: bool,
    /// Remaining steps to execute (front = next step).
    pub steps: VecDeque<DoorPassStep>,
    /// Order identities allocated eagerly by PassDoor translation, in the
    /// same order as `steps`. Original constructs the complete translated
    /// order list up front even though Rust executes its steps lazily.
    #[serde(default)]
    pub preallocated_order_ids: VecDeque<Option<std::num::NonZeroU32>>,
    /// How many PassingDoor triggers have fired (first changes layer, second
    /// re-enables anti-collision).
    pub triggers_fired: u8,
    /// Animation for the currently executing Walk step. Set when a Walk step
    /// is popped, read by `tick_entity_movement` for sprite animation.
    pub current_action: crate::order::OrderType,
    /// Whether the current Walk step plays its animation in reverse.
    pub current_reverse: bool,
    /// When a `Transition` step is popped and its animation starts via
    /// `active_ai_anim`, the actor's walking `action_state` is saved here
    /// and the runtime `action_state` is cleared to `Waiting` so the
    /// movement loop stops advancing.  When the animation completes and
    /// `advance_door_pass` proceeds to the next `Walk` step, the saved
    /// state is restored so movement resumes with the correct sprite row.
    /// The order list blocks naturally until the sprite animation
    /// reports `MOTION_TERMINATED`.
    pub saved_action_state: Option<ActionState>,
}

impl ActiveDoorPass {
    /// Restored passes may omit some trailing reserved identities. Preserve
    /// those as unallocated slots; extra identities have no corresponding step
    /// and are an invariant error. This does not allocate any order IDs.
    pub(crate) fn align_pending_order_ids(&mut self) {
        assert!(
            self.preallocated_order_ids.len() <= self.steps.len(),
            "door-pass order identity queue exceeds its translated step queue"
        );
        self.preallocated_order_ids.resize(self.steps.len(), None);
    }

    /// Fresh translation reserves the complete route before installing its
    /// first order, including steps which will only materialize later.
    pub(crate) fn preallocate_pending_order_ids(&mut self, next_order_id: &mut u32) {
        self.preallocated_order_ids = self
            .steps
            .iter()
            .map(|_| Some(crate::order::alloc_order_id(next_order_id)))
            .collect();
    }

    /// Consume one lazy step together with its reserved identity. The caller
    /// allocates an ID only when it materializes an unreserved step.
    pub(crate) fn pop_pending_step(
        &mut self,
    ) -> Option<(DoorPassStep, Option<std::num::NonZeroU32>)> {
        self.align_pending_order_ids();
        let step = self.steps.pop_front()?;
        let identity = self
            .preallocated_order_ids
            .pop_front()
            .expect("aligned door-pass step lost its identity slot");
        Some((step, identity))
    }

    /// Insert a newly translated step without displacing existing identities.
    pub(crate) fn insert_pending_step(
        &mut self,
        index: usize,
        step: DoorPassStep,
        order_id: std::num::NonZeroU32,
    ) {
        assert!(
            index <= self.steps.len(),
            "door-pass insertion index exceeds its step queue"
        );
        self.align_pending_order_ids();
        self.steps.insert(index, step);
        self.preallocated_order_ids.insert(index, Some(order_id));
    }

    /// Discard both halves when concrete order truncation removes the lazy tail.
    pub(crate) fn clear_pending_steps(&mut self) {
        self.steps.clear();
        self.preallocated_order_ids.clear();
    }
}

#[cfg(test)]
mod door_pass_pending_tests {
    use super::*;
    use std::num::NonZeroU32;

    fn pass() -> ActiveDoorPass {
        ActiveDoorPass {
            door_index: crate::gate::DoorIndex::new(67).unwrap(),
            direct: false,
            position_direct: true,
            steps: [
                DoorPassStep::Select { speed: 0.5 },
                DoorPassStep::PassingDoor,
            ]
            .into(),
            preallocated_order_ids: [NonZeroU32::new(41)].into(),
            triggers_fired: 1,
            current_action: crate::order::OrderType::WalkingUpright,
            current_reverse: false,
            saved_action_state: None,
        }
    }

    #[test]
    fn restored_partial_ids_stay_paired_through_insertion_and_consumption() {
        let mut pass = pass();
        pass.insert_pending_step(
            1,
            DoorPassStep::Select { speed: 0.75 },
            NonZeroU32::new(42).unwrap(),
        );
        assert_eq!(pass.steps.len(), 3);
        assert_eq!(
            pass.preallocated_order_ids,
            [NonZeroU32::new(41), NonZeroU32::new(42), None]
        );
        let (step, id) = pass.pop_pending_step().unwrap();
        assert!(matches!(step, DoorPassStep::Select { speed } if speed == 0.5));
        assert_eq!(id, NonZeroU32::new(41));
        let (step, id) = pass.pop_pending_step().unwrap();
        assert!(matches!(step, DoorPassStep::Select { speed } if speed == 0.75));
        assert_eq!(id, NonZeroU32::new(42));
        let (step, id) = pass.pop_pending_step().unwrap();
        assert!(matches!(step, DoorPassStep::PassingDoor));
        assert_eq!(
            id, None,
            "the restored unreserved step must not invent an ID"
        );
        assert!(pass.pop_pending_step().is_none());
        assert!(pass.preallocated_order_ids.is_empty());
        assert_eq!(
            pass.triggers_fired, 1,
            "queue changes cannot run door callbacks"
        );
        assert!(!pass.direct);
        assert!(pass.position_direct);
    }

    #[test]
    fn fresh_translation_reserves_ids_before_consuming_any_step() {
        let mut pass = pass();
        pass.preallocated_order_ids.clear();
        let mut next = 100;
        pass.preallocate_pending_order_ids(&mut next);
        assert_eq!(next, 102);
        assert_eq!(pass.pop_pending_step().unwrap().1, NonZeroU32::new(100));
        assert_eq!(pass.pop_pending_step().unwrap().1, NonZeroU32::new(101));
        assert_eq!(next, 102, "consumption must not allocate a second identity");
    }

    #[test]
    #[should_panic(expected = "door-pass order identity queue exceeds its translated step queue")]
    fn malformed_identity_tail_is_rejected_before_consumption() {
        let mut pass = pass();
        pass.preallocated_order_ids = [NonZeroU32::new(41); 3].into();
        pass.pop_pending_step();
    }

    #[test]
    fn discarded_route_clears_both_queues_without_allocating() {
        let mut pass = pass();
        pass.preallocated_order_ids = [NonZeroU32::new(41); 3].into();
        // Discard is intentionally unconditional, matching order truncation.
        pass.clear_pending_steps();
        assert!(pass.steps.is_empty());
        assert!(pass.preallocated_order_ids.is_empty());
    }

    #[test]
    #[should_panic(expected = "door-pass insertion index exceeds its step queue")]
    fn invalid_insertion_is_rejected() {
        let mut pass = pass();
        pass.insert_pending_step(3, DoorPassStep::PassingDoor, NonZeroU32::new(42).unwrap());
    }
}

/// Exact representation of the original game's installed actor order.
///
/// Sequence-manager selection is deliberately not sufficient: an element can
/// be selected before instruction installs its first order, while order advancement
/// can clear the pointer for the remainder of an actor slot even if later
/// manager work has already selected a fallback element.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct InstalledActorOrder {
    pub order_id: std::num::NonZeroU32,
    pub order_type: crate::order::OrderType,
}

/// Actor-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActorData {
    pub continuation: crate::actor_state::ActorContinuationState,
    pub old_action: Animation,
    pub is_ignored_for_anti_collision: bool,

    // Current state
    pub action_state: ActionState,
    pub execution_frozen: bool,
    pub sequence_element_started: bool,

    /// Actor-update-selected order identity, corresponding to original-game
    /// last-order identity. This is deliberately independent of the sprite's
    /// processed order: FrozenAll still consumes actor initialization once.
    pub last_execute_order_id: Option<std::num::NonZeroU32>,
    /// the original game's live order identity and action.
    pub installed_order: Option<InstalledActorOrder>,
    /// Original-game new-order state for the currently entered execution. Set at
    /// owner selection and cleared after Execute/completion/ActionChange.
    pub execute_order_initialising: bool,
    /// Orphaned WaitingSword identity retained across an already-satisfied
    /// EnterSwordfight terminal callback until the replacement Wait publishes.
    #[serde(default)]
    pub retained_waiting_sword_order_id: Option<std::num::NonZeroU32>,

    // Wait
    pub wait_time: u32,

    /// Countdown for the Listen ability's one-shot reveal.  Armed to
    /// `TIME_LISTEN_WAIT` (25) by the ai.rs section 2a listen pass
    /// when the PC enters the `ListenPhase::CountingDown` phase, then
    /// decremented each subsequent frame.  When it reaches 0, the blip
    /// reveal + FX target `Heard()` callback fires exactly once and
    /// the phase advances to `ExitTransition`.
    pub listen_wait_time: u32,

    /// Countdown for the Whistle ability's expanding-noise ellipse
    /// render pass.  Armed to `TIME_LISTEN_WAIT` (25) on the first
    /// tick of the whistle animation, decremented each subsequent
    /// frame until 0.  Read by `render_listen_ping` to draw the
    /// expanding circle during the last `TIME_LISTEN` (5) frames (the
    /// Whistling arm of the shared Listen/Whistle ellipse render).
    pub whistle_wait_time: u32,

    /// Current phase of the Listen ability, if any.  We carry an
    /// explicit phase so owner-local ability animation and countdown work
    /// can coordinate
    /// without re-parsing order types every frame.
    pub listen_phase: ListenPhase,

    /// Current phase of the beggar's `ReceivePurse` animation chain, if
    /// any.  The three-order queue `ReceivingPurse → WaitingWithPurse →
    /// Transition` is driven phase-by-phase so the `WaitingWithPurse`
    /// completion can fire `EngineInner::reveal_scrolls` at the right
    /// moment.
    pub receive_purse_phase: ReceivePursePhase,

    // Seeking
    pub seek_target: Option<EntityId>,
    pub last_seek_target_position: MapPoint,
    /// Original-game actor seek distance: the unadapted distance
    /// requested by the transient Seek command. Moving-target refreshes
    /// derive their concrete tolerance from this stable base each time.
    pub seek_distance: f32,
    /// Countdown before the actor may re-issue a seek against a moving
    /// target.  Armed to `TIME_SEEK_REFRESH` (25) at seek launch and
    /// after each seek refresh; decremented by entity-target seeking
    /// owner execution when that call does not return through a successful
    /// post-seek arrival.
    pub seek_refresh_wait: u32,
    // Note: concrete seek tolerance/flags/sector/layer live on the active
    // `Movement` element. `seek_distance` is deliberately actor-owned
    // because seek refresh repeatedly derives concrete moving-target
    // tolerances from the original, unadapted request.
    /// Post-seek sequence launched via `SEQ_INFO` when the seek ends
    /// (target reached/lost, or a self-seek collapses immediately).
    /// Copied from the movement sequence element's `post_seek_sequence`
    /// at seek dispatch.
    pub post_seek_sequence: Option<crate::sequence::PostSeekSequence>,

    pub passing_door_directly: bool,

    pub script_class: String,

    /// Tracks the sequence element that initiated the current movement,
    /// so we can notify the sequence manager when movement completes.
    pub active_movement: ActiveMovement,

    /// Multi-step door-pass state. When set, the movement tick processes
    /// steps one at a time: walk steps set waypoints, PassingDoor steps
    /// fire the layer/sector callback, and Transition steps play
    /// animations in place. See [`ActiveDoorPass`].
    pub active_door_pass: Option<ActiveDoorPass>,

    /// Tracks the sequence element that initiated an in-progress ranged
    /// action (currently only bow shots).  See
    /// [`ActiveShot`][crate::movement::ActiveShot] for details.
    pub active_shot: ActiveShot,

    /// Tracks an in-progress hero ability (carry, tie, heal, whistle, etc.).
    /// See [`ActiveAbility`][crate::movement::ActiveAbility] for details.
    pub active_ability: crate::movement::ActiveAbility,

    /// Per-frame sweep state for lateral/circle strikes.
    /// Initialized at the hit frame; cleared when the melee strike ends.
    pub sweep_state: Option<crate::movement::SweepState>,

    /// Victims to enter sword-fight with when the current push strike
    /// finishes. Populated at the push-strike hit frame (MOTION_DONE) and
    /// drained when the strike terminates (MOTION_TERMINATED).
    /// The victim list launches `EnterSwordfight` at terminate time,
    /// not at hit time.
    pub pending_push_swordfight: Vec<EntityId>,

    /// Destination point for rolling after a death/knockout fall on a slope.
    /// When `combat_anim` finishes and this is set, a Rolling animation is
    /// queued toward this point.
    pub pending_roll: Option<MapPoint>,

    /// World position the shield should face toward during movement.
    /// Set by `dispatch_raise_shield` from the danger point; cleared on
    /// shield lower or combat exit.  Faces the shield toward the threat
    /// rather than the opponent.
    pub shield_face_point: Option<MapPoint>,

    // -- Push flight state --
    /// Active push-flight.  When `Some`, the entity is being pushed through
    /// the air by a push/circle/charge strike.  Each frame the position
    /// advances by the stored increment.
    pub active_flight: Option<ActiveFlight>,

    // -- Lift climb state --
    /// If the actor currently owns a ladder-lift reservation, which sector
    /// and which direction. Set at WAIT_FREE_LIFT entry (wall routes do not
    /// contain that action), cleared on the corresponding door-pass exit. Used by
    /// `translate_ladder_wall_fall` to decrement the sector occupancy
    /// counter when a climber gets shoved off.
    pub active_lift: Option<ActiveLiftClimb>,

    // -- Rider charge state --
    /// Active rider charge state.  When `Some`, the rider is executing
    /// Rider-charge execution — moving along a path while checking a
    /// polygon hit zone each frame.
    pub active_rider_charge: Option<ActiveRiderCharge>,

    /// Actor-level identity of the last `RiderCharging` order whose
    /// rider-charge pass completed. This is intentionally distinct
    /// from `Sprite::last_processed_order_id`: FrozenAll still executes the
    /// charge polygon but must leave sprite motion initialization pending.
    pub last_executed_rider_charge_order_id: Option<std::num::NonZeroU32>,

    /// 3D bounding-box obstacle representing the shield held in front of
    /// this actor. Refreshed only at the original game's explicit shield-update
    /// call sites and retained between them. Used by `tick_arrows` to block
    /// incoming arrows.
    ///
    pub shield_obstacle: Option<crate::sight_obstacle::SightObstacle>,

    /// Active line-jump state.  Populated by
    /// [`EngineInner::start_jump`](crate::engine::EngineInner::start_jump) and
    /// drained by [`EngineInner::tick_active_jump_for`]; the actor is
    /// position-driven by the jump module while this is `Some`.
    pub active_jump: Option<crate::engine::jump::ActiveJump>,
    /// Target 3D point of the currently-executing jump step.  Stashed
    /// here so the flight can interpolate toward it on each frame
    /// without re-peeking the consumed step.
    pub active_jump_target_3d: Option<WorldPoint3D>,
    /// Whether the currently-executing jump step is airborne (drives
    /// `jump_z_offset` during interpolation).
    pub active_jump_airborne: bool,
    /// Visual lift applied to the sprite during airborne jump steps.
    /// The renderer subtracts this from the sprite's world Y so the
    /// character appears above the ground.  `0.0` on the ground.
    pub jump_z_offset: f32,

    /// Last computed produced-noise volume.  Persists across frames
    /// to implement the `RHMATERIAL_LIGHT_SHADOW` carry-over in
    /// `refresh_produced_noise`, where walks/runs on light-shadow keep
    /// the previous frame's volume.
    pub last_noise_volume: u16,
    /// Persistent human-actor noise-hearing box state.
    ///
    /// The original game updates the noise origin before its
    /// animation switch, but its inactive/building and quiet-animation arms
    /// return before rebuilding this box.  Hearing therefore deliberately
    /// tests a stale box in those cases instead of deriving one from the
    /// current volume and origin.
    pub hear_noise_box: MapBBox,
    /// Complete produced-noise record from this PC's most recently visited
    /// human update slot. NPCs earlier in creation order observe this
    /// prior record; later NPCs observe the freshly replaced record.
    pub produced_noise: Option<crate::ai::Noise>,
}

impl Default for ActorData {
    fn default() -> Self {
        Self {
            continuation: crate::actor_state::ActorContinuationState::default(),
            old_action: Animation::default(),
            is_ignored_for_anti_collision: false,
            action_state: ActionState::default(),
            execution_frozen: false,
            sequence_element_started: false,
            last_execute_order_id: None,
            installed_order: None,
            execute_order_initialising: false,
            retained_waiting_sword_order_id: None,
            wait_time: 0,
            listen_wait_time: 0,
            whistle_wait_time: 0,
            listen_phase: ListenPhase::Inactive,
            receive_purse_phase: ReceivePursePhase::Inactive,
            seek_target: None,
            last_seek_target_position: MapPoint::default(),
            seek_distance: 0.0,
            seek_refresh_wait: 0,
            post_seek_sequence: None,
            passing_door_directly: false,
            script_class: String::new(),
            active_movement: ActiveMovement::none(),
            active_door_pass: None,
            active_shot: ActiveShot::none(),
            active_ability: crate::movement::ActiveAbility::default(),
            sweep_state: None,
            pending_push_swordfight: Vec::new(),
            pending_roll: None,
            shield_face_point: None,
            active_jump: None,
            active_jump_target_3d: None,
            active_jump_airborne: false,
            jump_z_offset: 0.0,
            active_flight: None,
            active_lift: None,
            active_rider_charge: None,
            last_executed_rider_charge_order_id: None,
            shield_obstacle: None,
            last_noise_volume: 0,
            hear_noise_box: MapBBox::new(),
            produced_noise: None,
        }
    }
}

impl ActorData {
    /// Publish the actor Execute selection even while execution is frozen.
    /// Installed/sprite order identity is deliberately not derived from this
    /// latch: those are published at different owner boundaries.
    pub(crate) fn select_execute_order(&mut self, order_id: std::num::NonZeroU32) {
        self.execute_order_initialising = self.last_execute_order_id != Some(order_id);
        self.last_execute_order_id = Some(order_id);
    }

    /// StartPostSeekSequence's teardown before an out-of-range PC Hit aborts.
    /// The overloaded wait value is folded before the invalid interaction is
    /// reported. Preserve the seek distance/last position and independent
    /// order, action, jump, and ability latches for their owning callbacks.
    pub(crate) fn abort_out_of_range_hit_seek(&mut self) {
        self.wait_time = self.seek_refresh_wait;
        self.seek_target = None;
        self.post_seek_sequence = None;
        self.clear_path();
        self.active_door_pass = None;
    }

    /// Install a translated jump without advancing its first Execute slot.
    /// The outgoing action and airborne latches deliberately survive until
    /// that slot; they are independently observable Original state.
    pub(crate) fn install_jump(
        &mut self,
        jump: crate::engine::jump::ActiveJump,
        first_order: InstalledActorOrder,
    ) {
        self.clear_path();
        self.active_jump = Some(jump);
        self.jump_z_offset = 0.0;
        self.installed_order = Some(first_order);
    }

    /// Release completed jump ownership and visual lift. Do not erase the
    /// last step's target/airborne latches or choose the next action here:
    /// landing callbacks and the selected-order path own those boundaries.
    pub(crate) fn finish_jump(&mut self) {
        self.active_jump = None;
        self.jump_z_offset = 0.0;
    }

    /// Publish a selected step and its movement latches together. The caller
    /// must already have selected a live jump owner.
    pub(crate) fn install_jump_step(&mut self, state: crate::engine::jump::CurrentStepState) {
        let jump = self
            .active_jump
            .as_mut()
            .expect("selected jump step has no live jump owner");
        self.active_jump_target_3d = state.step.target_3d;
        self.active_jump_airborne = state.step.airborne;
        jump.current = Some(state);
    }

    /// Decouple this actor from its active Move element.  Legacy
    /// name for "stop this actor moving now" — most ability-setup
    /// sites call this before installing their own sequence element,
    /// and priority arbitration will interrupt the orphaned Move
    /// soon after.  For a hard teardown (terminate the Move
    /// immediately so its remaining orders can't animate), use
    /// `EngineInner::abort_actor_movement` instead.
    pub fn clear_path(&mut self) {
        self.active_movement.clear();
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SmalltalkHint {
    #[default]
    None,
    Left,
    Right,
    Legs,
}

/// Exact mutable repulsive-point state owned by a human.
///
/// The normal anti-collision projection is rebuilt from posture every frame,
/// but Original saves retain the complete owner object.  Keeping the complete
/// state here lets a mid-frame load resume before that refresh without
/// inventing geometry or dropping the Original affect-mask.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanRepulsivePointState {
    pub position: MapPoint,
    pub concave: bool,
    pub limit_left: MapPoint,
    pub limit_right: MapPoint,
    pub action_radius: f32,
    pub force_a: f32,
    pub force_b: f32,
    pub radius: f32,
    pub id: u32,
    pub affects_pcs: bool,
    pub affects_soldiers: bool,
    pub affects_civilians: bool,
    pub affects_animals: bool,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanShieldPointState {
    pub obstacle: [f32; 4],
    pub polygon: MapPoint,
}

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanPlaneState {
    pub a: WorldPoint3D,
    pub b: WorldPoint3D,
    pub normal: WorldPoint3D,
    pub origin: WorldPoint3D,
    pub u: WorldPoint3D,
    pub v: WorldPoint3D,
    pub az: f32,
    pub bz: f32,
    pub dz: f32,
    pub d: f32,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanBoundingBox2State {
    pub top_left: MapPoint,
    pub bottom_right: MapPoint,
    pub bounds_are_set: bool,
}

/// Exact serialized sight obstacle owned by a human.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanShieldState {
    pub points: [HumanShieldPointState; 4],
    pub top_plane: HumanPlaneState,
    pub bottom_plane: HumanPlaneState,
    pub box_3d: [f32; 6],
    pub ground_box: HumanBoundingBox2State,
    pub screen_box: HumanBoundingBox2State,
    pub on_ground: bool,
}

/// Serialized progress of a Human's in-flight multi-victim sword strike.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanSwordSweepState {
    pub victims: Vec<EntityId>,
    pub initial_angle: f32,
    pub current_angle: f32,
    pub final_angle: f32,
}

/// One entry in the original game's ordered swordfight-opponent list.
///
/// The jump line belongs to this human's side of a table swordfight. Keeping
/// it in the same record as the opponent prevents principal promotion,
/// removal, and insertion from desynchronizing the two values.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct SwordfightOpponent {
    opponent: EntityId,
    jump_line: Option<JumpLineIndex>,
}

impl SwordfightOpponent {
    /// Build one paired opponent record.
    ///
    /// The jump line is the line on this human's side of the fight, matching
    /// Original-game swordfight-opponent jump line.
    pub fn new(opponent: EntityId, jump_line: Option<JumpLineIndex>) -> Self {
        Self {
            opponent,
            jump_line,
        }
    }

    pub fn opponent(self) -> EntityId {
        self.opponent
    }

    pub fn jump_line(self) -> Option<JumpLineIndex> {
        self.jump_line
    }
}

/// Ordered swordfight opponents; the first entry is the principal opponent.
///
/// The original game stores a single swordfight-opponent list. Older Rust
/// snapshots exposed the two record fields as parallel `opponents` and
/// `opponent_jump_lines` vectors. [`HumanData`]'s compatibility view and this
/// type's `StateHash` implementation retain that wire shape and hash byte
/// order while the live representation enforces the Original's one-record
/// invariant.
#[derive(
    Clone, Default, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(transparent)]
pub struct SwordfightOpponents {
    entries: Vec<SwordfightOpponent>,
}

impl SwordfightOpponents {
    /// Construct opponents without table jump lines, primarily for level
    /// loading and focused simulation fixtures.
    pub fn from_ids(ids: impl IntoIterator<Item = EntityId>) -> Self {
        Self {
            entries: ids
                .into_iter()
                .map(|opponent| SwordfightOpponent::new(opponent, None))
                .collect(),
        }
    }

    /// Construct an ordered list from already-paired records.
    pub fn from_entries(entries: impl IntoIterator<Item = SwordfightOpponent>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    pub(crate) fn from_pairs(
        pairs: impl IntoIterator<Item = (EntityId, Option<JumpLineIndex>)>,
    ) -> Self {
        Self::from_entries(
            pairs
                .into_iter()
                .map(|(opponent, jump_line)| SwordfightOpponent::new(opponent, jump_line)),
        )
    }

    fn try_from_parts(
        opponents: Vec<EntityId>,
        mut jump_lines: Vec<Option<JumpLineIndex>>,
    ) -> Result<Self, String> {
        if jump_lines.len() > opponents.len() {
            return Err(format!(
                "opponent_jump_lines has {} entries for {} opponents",
                jump_lines.len(),
                opponents.len()
            ));
        }

        // Historical Rust snapshots could have a short parallel vector. Its
        // read behavior was `get(index).flatten()`, i.e. missing meant no
        // jump line; normalize that representation at the boundary.
        jump_lines.resize(opponents.len(), None);
        Ok(Self::from_pairs(opponents.into_iter().zip(jump_lines)))
    }

    fn into_parts(self) -> (Vec<EntityId>, Vec<Option<JumpLineIndex>>) {
        let mut opponents = Vec::with_capacity(self.entries.len());
        let mut jump_lines = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            opponents.push(entry.opponent);
            jump_lines.push(entry.jump_line);
        }
        (opponents, jump_lines)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn first(&self) -> Option<&EntityId> {
        self.entries.first().map(|entry| &entry.opponent)
    }

    pub fn get(&self, index: usize) -> Option<&EntityId> {
        self.entries.get(index).map(|entry| &entry.opponent)
    }

    pub fn contains(&self, opponent: &EntityId) -> bool {
        self.entries.iter().any(|entry| entry.opponent == *opponent)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &EntityId> {
        self.entries.iter().map(|entry| &entry.opponent)
    }

    pub fn iter_with_jump_lines(
        &self,
    ) -> impl ExactSizeIterator<Item = (EntityId, Option<JumpLineIndex>)> + '_ {
        self.entries
            .iter()
            .map(|entry| (entry.opponent, entry.jump_line))
    }

    pub fn jump_line(&self, index: usize) -> Option<JumpLineIndex> {
        self.entries.get(index).and_then(|entry| entry.jump_line)
    }

    pub fn ids(&self) -> Vec<EntityId> {
        self.iter().copied().collect()
    }

    /// Match the original game: insert a new principal opponent entry, or
    /// promote an existing non-principal entry and replace its jump line.
    pub(crate) fn add_principal(
        &mut self,
        opponent: EntityId,
        jump_line: Option<JumpLineIndex>,
    ) -> bool {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.opponent == opponent)
        {
            if index != 0 {
                self.entries.swap(0, index);
                self.entries[0].jump_line = jump_line;
            }
            return false;
        }

        self.entries
            .insert(0, SwordfightOpponent::new(opponent, jump_line));
        true
    }

    pub(crate) fn remove(&mut self, opponent: EntityId) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.opponent == opponent)
        else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn promote(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.entries.swap(0, index);
        true
    }

    pub(crate) fn update_jump_line_at(
        &mut self,
        index: usize,
        jump_line: Option<JumpLineIndex>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };
        entry.jump_line = jump_line;
        true
    }

    pub(crate) fn update_jump_line(
        &mut self,
        opponent: EntityId,
        jump_line: Option<JumpLineIndex>,
    ) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.opponent == opponent)
        else {
            return false;
        };
        entry.jump_line = jump_line;
        true
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, opponent: EntityId) {
        self.entries.push(SwordfightOpponent::new(opponent, None));
    }

    #[cfg(test)]
    pub(crate) fn extend(&mut self, opponents: impl IntoIterator<Item = EntityId>) {
        self.entries.extend(
            opponents
                .into_iter()
                .map(|opponent| SwordfightOpponent::new(opponent, None)),
        );
    }
}

impl From<Vec<EntityId>> for SwordfightOpponents {
    fn from(opponents: Vec<EntityId>) -> Self {
        Self::from_ids(opponents)
    }
}

impl fmt::Debug for SwordfightOpponents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for &'a SwordfightOpponents {
    type Item = &'a EntityId;
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, SwordfightOpponent>,
        fn(&'a SwordfightOpponent) -> &'a EntityId,
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn opponent(entry: &SwordfightOpponent) -> &EntityId {
            &entry.opponent
        }
        self.entries.iter().map(opponent)
    }
}

impl std::ops::Index<usize> for SwordfightOpponents {
    type Output = EntityId;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index].opponent
    }
}

impl PartialEq<Vec<EntityId>> for SwordfightOpponents {
    fn eq(&self, other: &Vec<EntityId>) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl PartialEq<SwordfightOpponents> for Vec<EntityId> {
    fn eq(&self, other: &SwordfightOpponents) -> bool {
        other == self
    }
}

impl robin_util::state_hash::StateHash for SwordfightOpponents {
    fn state_hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.entries.len() as u64);
        for entry in &self.entries {
            robin_util::state_hash::StateHash::state_hash(&entry.opponent, state);
        }
        state.write_u64(self.entries.len() as u64);
        for entry in &self.entries {
            robin_util::state_hash::StateHash::state_hash(&entry.jump_line, state);
        }
    }
}

/// Human-level data.
#[derive(Debug, Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)]
pub struct HumanData {
    pub carrier: Option<EntityId>,

    // Health & combat
    pub concussion_of_the_brain: u16,
    pub concussion_healing_timeout: u16,
    pub tiredness: u16,
    pub unconscious: bool,
    pub already_detectable_body: bool,
    pub detectable_list_index: u16,

    // Sword strikes
    pub sword_strike_boredom: Vec<u16>,

    // Nets
    pub stuck_under_nets_counter: u16,

    // Visibility
    pub hollow_man: bool,

    // Swordfight — the Original's ordered opponent records.
    pub opponents: SwordfightOpponents,

    pub smalltalk_initiative: bool,
    pub received_smalltalk_initiative: bool,
    pub smalltalk_hint: SmalltalkHint,
    pub smalltalk_hint_opponent: Option<EntityId>,
    pub relative_fighting_ability: u16,

    // Shield & combat
    pub small_repulsive_radius: bool,
    /// Previously-observed `posture.is_lying()` state for
    /// [`EngineInner::process_corpse_intersection_updates`].
    ///
    /// `None` until the first observation (fresh spawn or post-load);
    /// that first tick seeds it without firing an update so the
    /// serialized `small_repulsive_radius` flag stays authoritative.
    /// Later a mismatch against the current posture drives the
    /// engine-level `update_intersecting_corpses` hook.
    pub last_is_lying_for_corpse_intersection: Option<bool>,
    pub killed_by_accident: bool,
    pub parry_counter: u16,
    pub invulnerable: bool,
    pub last_motion_was_step_back_in_combat: bool,

    // Hulk glow effect
    pub running_hulk: u32,
    pub time_hulk: u32,
    pub hulk_level: u16,
    pub hulk_direction: bool,
    pub hulk_speed: f32,

    /// Original owner-local state which is either consumed before, or rebuilt
    /// by, the corresponding Rust subsystem after a loaded frame.
    pub repulsive_point: HumanRepulsivePointState,
    pub building_sector: Option<SectorHandle>,
    /// The original game's enum serialization accidentally
    /// serialized only the first word (`posOrigin.x`).  The rest is refreshed
    /// by Human noise production and is intentionally not fabricated.
    pub produced_noise_first_word: f32,
    pub shield: HumanShieldState,
    pub sword_sweep: HumanSwordSweepState,
    pub pending_shoots: Vec<crate::sequence::SequenceElementRef>,
}

/// Compatibility view of [`HumanData`]. The two opponent vectors deliberately
/// remain adjacent and in their historical order for JSON saves.
#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct HumanDataWire {
    carrier: Option<EntityId>,
    concussion_of_the_brain: u16,
    concussion_healing_timeout: u16,
    tiredness: u16,
    unconscious: bool,
    already_detectable_body: bool,
    detectable_list_index: u16,
    sword_strike_boredom: Vec<u16>,
    stuck_under_nets_counter: u16,
    hollow_man: bool,
    opponents: Vec<EntityId>,
    #[serde(default)]
    opponent_jump_lines: Vec<Option<JumpLineIndex>>,
    smalltalk_initiative: bool,
    received_smalltalk_initiative: bool,
    smalltalk_hint: SmalltalkHint,
    smalltalk_hint_opponent: Option<EntityId>,
    relative_fighting_ability: u16,
    small_repulsive_radius: bool,
    last_is_lying_for_corpse_intersection: Option<bool>,
    killed_by_accident: bool,
    parry_counter: u16,
    invulnerable: bool,
    last_motion_was_step_back_in_combat: bool,
    running_hulk: u32,
    time_hulk: u32,
    hulk_level: u16,
    hulk_direction: bool,
    hulk_speed: f32,
    repulsive_point: HumanRepulsivePointState,
    building_sector: Option<SectorHandle>,
    produced_noise_first_word: f32,
    shield: HumanShieldState,
    sword_sweep: HumanSwordSweepState,
    pending_shoots: Vec<crate::sequence::SequenceElementRef>,
}

struct SerializedOpponentIds<'a>(&'a SwordfightOpponents);

impl Serialize for SerializedOpponentIds<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;

        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for opponent in self.0.iter() {
            sequence.serialize_element(opponent)?;
        }
        sequence.end()
    }
}

struct SerializedOpponentJumpLines<'a>(&'a SwordfightOpponents);

impl Serialize for SerializedOpponentJumpLines<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;

        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in &self.0.entries {
            sequence.serialize_element(&entry.jump_line)?;
        }
        sequence.end()
    }
}

#[derive(Serialize)]
struct HumanDataWireRef<'a> {
    carrier: &'a Option<EntityId>,
    concussion_of_the_brain: &'a u16,
    concussion_healing_timeout: &'a u16,
    tiredness: &'a u16,
    unconscious: &'a bool,
    already_detectable_body: &'a bool,
    detectable_list_index: &'a u16,
    sword_strike_boredom: &'a [u16],
    stuck_under_nets_counter: &'a u16,
    hollow_man: &'a bool,
    opponents: SerializedOpponentIds<'a>,
    opponent_jump_lines: SerializedOpponentJumpLines<'a>,
    smalltalk_initiative: &'a bool,
    received_smalltalk_initiative: &'a bool,
    smalltalk_hint: &'a SmalltalkHint,
    smalltalk_hint_opponent: &'a Option<EntityId>,
    relative_fighting_ability: &'a u16,
    small_repulsive_radius: &'a bool,
    last_is_lying_for_corpse_intersection: &'a Option<bool>,
    killed_by_accident: &'a bool,
    parry_counter: &'a u16,
    invulnerable: &'a bool,
    last_motion_was_step_back_in_combat: &'a bool,
    running_hulk: &'a u32,
    time_hulk: &'a u32,
    hulk_level: &'a u16,
    hulk_direction: &'a bool,
    hulk_speed: &'a f32,
    repulsive_point: &'a HumanRepulsivePointState,
    building_sector: &'a Option<SectorHandle>,
    produced_noise_first_word: &'a f32,
    shield: &'a HumanShieldState,
    sword_sweep: &'a HumanSwordSweepState,
    pending_shoots: &'a [crate::sequence::SequenceElementRef],
}

impl Serialize for HumanData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        HumanDataWireRef {
            carrier: &self.carrier,
            concussion_of_the_brain: &self.concussion_of_the_brain,
            concussion_healing_timeout: &self.concussion_healing_timeout,
            tiredness: &self.tiredness,
            unconscious: &self.unconscious,
            already_detectable_body: &self.already_detectable_body,
            detectable_list_index: &self.detectable_list_index,
            sword_strike_boredom: &self.sword_strike_boredom,
            stuck_under_nets_counter: &self.stuck_under_nets_counter,
            hollow_man: &self.hollow_man,
            opponents: SerializedOpponentIds(&self.opponents),
            opponent_jump_lines: SerializedOpponentJumpLines(&self.opponents),
            smalltalk_initiative: &self.smalltalk_initiative,
            received_smalltalk_initiative: &self.received_smalltalk_initiative,
            smalltalk_hint: &self.smalltalk_hint,
            smalltalk_hint_opponent: &self.smalltalk_hint_opponent,
            relative_fighting_ability: &self.relative_fighting_ability,
            small_repulsive_radius: &self.small_repulsive_radius,
            last_is_lying_for_corpse_intersection: &self.last_is_lying_for_corpse_intersection,
            killed_by_accident: &self.killed_by_accident,
            parry_counter: &self.parry_counter,
            invulnerable: &self.invulnerable,
            last_motion_was_step_back_in_combat: &self.last_motion_was_step_back_in_combat,
            running_hulk: &self.running_hulk,
            time_hulk: &self.time_hulk,
            hulk_level: &self.hulk_level,
            hulk_direction: &self.hulk_direction,
            hulk_speed: &self.hulk_speed,
            repulsive_point: &self.repulsive_point,
            building_sector: &self.building_sector,
            produced_noise_first_word: &self.produced_noise_first_word,
            shield: &self.shield,
            sword_sweep: &self.sword_sweep,
            pending_shoots: &self.pending_shoots,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HumanData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        HumanData::try_from(HumanDataWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl TryFrom<HumanDataWire> for HumanData {
    type Error = String;

    fn try_from(wire: HumanDataWire) -> Result<Self, Self::Error> {
        let opponents =
            SwordfightOpponents::try_from_parts(wire.opponents, wire.opponent_jump_lines)?;
        Ok(Self {
            carrier: wire.carrier,
            concussion_of_the_brain: wire.concussion_of_the_brain,
            concussion_healing_timeout: wire.concussion_healing_timeout,
            tiredness: wire.tiredness,
            unconscious: wire.unconscious,
            already_detectable_body: wire.already_detectable_body,
            detectable_list_index: wire.detectable_list_index,
            sword_strike_boredom: wire.sword_strike_boredom,
            stuck_under_nets_counter: wire.stuck_under_nets_counter,
            hollow_man: wire.hollow_man,
            opponents,
            smalltalk_initiative: wire.smalltalk_initiative,
            received_smalltalk_initiative: wire.received_smalltalk_initiative,
            smalltalk_hint: wire.smalltalk_hint,
            smalltalk_hint_opponent: wire.smalltalk_hint_opponent,
            relative_fighting_ability: wire.relative_fighting_ability,
            small_repulsive_radius: wire.small_repulsive_radius,
            last_is_lying_for_corpse_intersection: wire.last_is_lying_for_corpse_intersection,
            killed_by_accident: wire.killed_by_accident,
            parry_counter: wire.parry_counter,
            invulnerable: wire.invulnerable,
            last_motion_was_step_back_in_combat: wire.last_motion_was_step_back_in_combat,
            running_hulk: wire.running_hulk,
            time_hulk: wire.time_hulk,
            hulk_level: wire.hulk_level,
            hulk_direction: wire.hulk_direction,
            hulk_speed: wire.hulk_speed,
            repulsive_point: wire.repulsive_point,
            building_sector: wire.building_sector,
            produced_noise_first_word: wire.produced_noise_first_word,
            shield: wire.shield,
            sword_sweep: wire.sword_sweep,
            pending_shoots: wire.pending_shoots,
        })
    }
}

impl From<HumanData> for HumanDataWire {
    fn from(human: HumanData) -> Self {
        let (opponents, opponent_jump_lines) = human.opponents.into_parts();
        Self {
            carrier: human.carrier,
            concussion_of_the_brain: human.concussion_of_the_brain,
            concussion_healing_timeout: human.concussion_healing_timeout,
            tiredness: human.tiredness,
            unconscious: human.unconscious,
            already_detectable_body: human.already_detectable_body,
            detectable_list_index: human.detectable_list_index,
            sword_strike_boredom: human.sword_strike_boredom,
            stuck_under_nets_counter: human.stuck_under_nets_counter,
            hollow_man: human.hollow_man,
            opponents,
            opponent_jump_lines,
            smalltalk_initiative: human.smalltalk_initiative,
            received_smalltalk_initiative: human.received_smalltalk_initiative,
            smalltalk_hint: human.smalltalk_hint,
            smalltalk_hint_opponent: human.smalltalk_hint_opponent,
            relative_fighting_ability: human.relative_fighting_ability,
            small_repulsive_radius: human.small_repulsive_radius,
            last_is_lying_for_corpse_intersection: human.last_is_lying_for_corpse_intersection,
            killed_by_accident: human.killed_by_accident,
            parry_counter: human.parry_counter,
            invulnerable: human.invulnerable,
            last_motion_was_step_back_in_combat: human.last_motion_was_step_back_in_combat,
            running_hulk: human.running_hulk,
            time_hulk: human.time_hulk,
            hulk_level: human.hulk_level,
            hulk_direction: human.hulk_direction,
            hulk_speed: human.hulk_speed,
            repulsive_point: human.repulsive_point,
            building_sector: human.building_sector,
            produced_noise_first_word: human.produced_noise_first_word,
            shield: human.shield,
            sword_sweep: human.sword_sweep,
            pending_shoots: human.pending_shoots,
        }
    }
}

impl Default for HumanData {
    fn default() -> Self {
        Self {
            carrier: None,
            concussion_of_the_brain: 0,
            concussion_healing_timeout: 0,
            tiredness: 0,
            unconscious: false,
            already_detectable_body: false,
            detectable_list_index: 0,
            sword_strike_boredom: Vec::new(),
            stuck_under_nets_counter: 0,
            hollow_man: false,
            opponents: SwordfightOpponents::default(),
            smalltalk_initiative: false,
            received_smalltalk_initiative: false,
            smalltalk_hint: SmalltalkHint::None,
            smalltalk_hint_opponent: None,
            relative_fighting_ability: 0,
            small_repulsive_radius: false,
            last_is_lying_for_corpse_intersection: None,
            killed_by_accident: false,
            parry_counter: 0,
            invulnerable: false,
            last_motion_was_step_back_in_combat: false,
            running_hulk: 0,
            time_hulk: 0,
            hulk_level: 0,
            hulk_direction: false,
            hulk_speed: 1.0,
            repulsive_point: HumanRepulsivePointState::default(),
            building_sector: None,
            produced_noise_first_word: 0.0,
            shield: HumanShieldState::default(),
            sword_sweep: HumanSwordSweepState::default(),
            pending_shoots: Vec::new(),
        }
    }
}

/// Default hulk animation length in frames.
pub const HULK_LENGTH: u32 = 20;

impl HumanData {
    /// Start the hulk outline glow animation.
    pub fn start_hulk(&mut self, fade_out: bool, speed: f32) {
        self.hulk_direction = fade_out;
        self.time_hulk = (speed * HULK_LENGTH as f32) as u32;
        self.running_hulk = self.time_hulk;
    }
}

/// Ammo counters owned by a live PC entity.
///
/// An original-game player actor always has a player-status reference, and
/// `SetPersistentProperty` mutates that live status directly.  Campaign mode
/// also persists the same values in [`crate::campaign::PcDescription`], but a
/// script call must not depend on a campaign object being installed.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcAmmoData {
    pub ales: u16,
    pub arrows: u16,
    pub apples: u16,
    pub rations: u16,
    pub stones: u16,
    pub wasp_nests: u16,
    pub nets: u16,
    pub plants: u16,
    pub purses: u16,
}

impl PcAmmoData {
    pub fn get(&self, action: Action) -> Option<u16> {
        match action {
            Action::Ale => Some(self.ales),
            Action::Apple => Some(self.apples),
            Action::Bow => Some(self.arrows),
            Action::Eat | Action::Guzzle => Some(self.rations),
            Action::Net => Some(self.nets),
            Action::Stone => Some(self.stones),
            Action::Heal => Some(self.plants),
            Action::Purse => Some(self.purses),
            Action::WaspNest => Some(self.wasp_nests),
            _ => None,
        }
    }

    pub fn set(&mut self, action: Action, quantity: u16) -> Option<()> {
        let counter = match action {
            Action::Ale => &mut self.ales,
            Action::Apple => &mut self.apples,
            Action::Bow => &mut self.arrows,
            Action::Eat | Action::Guzzle => &mut self.rations,
            Action::Net => &mut self.nets,
            Action::Stone => &mut self.stones,
            Action::Heal => &mut self.plants,
            Action::Purse => &mut self.purses,
            Action::WaspNest => &mut self.wasp_nests,
            _ => return None,
        };
        *counter = quantity;
        Some(())
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    Default,
)]
pub struct PcPortraitQuickIconState {
    #[serde(deserialize_with = "Option::deserialize")]
    pub titbit_id: Option<crate::titbit::TitbitId>,
    pub running: bool,
}

/// Engine-owned copy of the Original portrait state needed by save adoption.
///
/// The renderer may project this into a host widget, but simulation adoption
/// must not lose it merely because no widget exists in a headless replay.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcPortraitState {
    pub quantities: [u16; 3],
    pub two_buttons_mode: bool,
    pub displayed: bool,
    pub burned: bool,
    pub open: bool,
    pub life_level: f32,
    pub trumpet_enabled: bool,
    pub quick_icons: [PcPortraitQuickIconState; 3],
}

fn default_pc_camp() -> Camp {
    Camp::Royalists
}

const fn default_hero_command_interface() -> CommandInterface {
    CommandInterface::HeroActions
}

const fn default_hero_mission_role() -> MissionRole {
    MissionRole::PlayerParty
}

/// PC-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcData {
    /// Life points stored directly.
    pub life_points: i16,
    pub immortal: bool,
    pub robin: bool,
    pub already_selected: bool,
    pub list_index: u8,
    /// Mission-authored allegiance. Ordinary campaign PCs use the player
    /// allegiance assigned during construction.
    #[serde(default = "default_pc_camp")]
    pub cached_camp: Camp,
    /// Exact index of the original game's description reference in
    /// Campaign player-character description map.
    ///
    /// This is independent of the list index: multiple campaign
    /// descriptions may share one character profile, while the list byte is
    /// actor/UI state serialized separately for the player actor.
    #[serde(default)]
    pub campaign_description_index: Option<u32>,

    /// Whether this PC can currently be selected and controlled.
    /// Set/cleared by the `Activate` and `Deactivate` script natives,
    /// and by rescue-PC spawn logic.
    pub playable: bool,
    /// Player command surface exposed by this hero. This is independent from
    /// the decision policy: a tactical unit can accept high-level player
    /// orders while retaining EnemyAi for moment-to-moment decisions.
    #[serde(default = "default_hero_command_interface")]
    pub command_interface: CommandInterface,
    /// Mission bookkeeping role, independent from allegiance and controller.
    #[serde(default = "default_hero_mission_role")]
    pub mission_role: MissionRole,
    /// Engagement policy used by whichever controller owns combat decisions.
    #[serde(default)]
    pub combat_stance: CombatStance,
    /// Shared AI runtime used by AI-controlled heroes. Ordinary directly
    /// controlled heroes intentionally have no AI owner.
    #[serde(default)]
    pub ai: Option<Box<AiActorData>>,

    /// Whether the per-PC UI panel should be hidden.  Toggled today by
    /// the `CALL <initial> HIDEINTERFACE|DISPLAYINTERFACE` console
    /// cheat.  The HUD port reads this flag when rendering.
    pub interface_hidden: bool,

    // Actions
    pub current_action: Action,
    pub saved_action: Action,
    pub disabled_actions: Vec<bool>,
    pub disabled_actions_temp: Vec<bool>,
    /// Live ammo state used by actor-local script behavior.  In campaign
    /// mode mutations are mirrored to the campaign character status.
    #[serde(default)]
    pub ammo: PcAmmoData,

    // Quick actions
    pub quick_action_types: Vec<QuickAction>,
    /// Stored sequences for each QA slot (up to 3). When the player
    /// replays a QA, the engine launches the sequence from this slot.
    pub quick_action_sequences: Vec<Option<crate::sequence::Sequence>>,
    pub quick_seek_sequences: Vec<Option<crate::sequence::Sequence>>,
    pub quick_action_special_counts: Vec<u16>,
    pub quick_action_buttons: Vec<u16>,
    pub quick_action_interactors: Vec<Option<EntityId>>,
    pub titbits: Vec<Option<crate::titbit::TitbitId>>,
    pub portrait: PcPortraitState,

    // Detection
    pub head_seen: bool,
    pub belt_seen: bool,
    pub feet_seen: bool,

    // Teleport
    pub position_before_teleport: MapPoint,
    /// Frames remaining in the cheat-teleport hulk-rebuild fade.
    /// Decremented each frame by the per-PC render path (not yet
    /// implemented); read here by teleport setup to suppress the
    /// old-position star burst when a re-teleport fires while the
    /// previous fade is still in flight.
    pub teleport_counter: u16,
    /// Initial value of [`Self::teleport_counter`] when the most recent
    /// teleport began.  Used by the render path to compute the fade
    /// percentage.
    pub max_teleport_counter: u16,
    pub fried_psykokwack: bool,

    // Carried person
    pub carried: Option<EntityId>,
    /// Raw original-game carried-posture storage.
    ///
    /// Original-game initialization leaves it indeterminate while `carried` is
    /// null. It is validated as a [`Posture`] whenever a carried body makes
    /// the field semantically live.
    pub carried_posture: u32,

    // Shield
    pub shield_danger_point: WorldPoint3D,
    /// Map layer the player picked when raising the shield, used as the
    /// layer for the danger-point titbit.  Differs from the PC's own
    /// layer when the danger is across a chasm / off a balcony.
    pub shield_danger_point_layer: u16,
    pub shield_protected: Option<EntityId>,
    pub shield_protector: Option<EntityId>,

    // Guard
    pub guard: Option<EntityId>,

    // Reinforcement
    pub time_till_reinforcement: u32,

    // Sherwood
    pub work_icon: WorkIcon,

    // Ammo dropping
    pub last_ammo_dropping_position: MapPoint,
    pub last_dropped_ammo: Option<EntityId>,
    pub update_last_dropped_ammo: bool,
    pub last_dropping_direction: u8,

    /// References character profile.
    pub profile_index: CharacterProfileIdx,
    /// Which of the 10 playable characters this PC represents.  `None`
    /// when level load encountered a character profile whose
    /// `profile_name` string isn't one of the known French names
    /// (mirrors the previous empty-string fallback).
    pub kind: Option<crate::character_kind::CharacterKind>,
    /// Cached contextual movement permissions from the character
    /// profile.  `disabled_actions` only tracks the three quick-action
    /// slots, not these profile-level abilities.
    pub has_lockpick: bool,
    pub has_climb: bool,
    pub has_jump: bool,

    /// Beam-me spawn index for Sherwood HQ positioning.
    /// -1 = not assigned. Set by engine during level setup.
    pub beam_me_index: i16,

    /// Whether the portrait's "trumpet" replacement-available indicator
    /// should be shown.  Set by the PC kill path when a non-VIP peasant
    /// is still available in the gang to replace the killed PC.
    pub trumpet_enabled: bool,

    /// The PC's current melee target (sword opponent).
    ///
    /// Set when the PC enters a swordfight, cleared when the fight
    /// ends.  Used to populate `FighterSnapshot.principal_opponent` so
    /// the enemy AI can reason about PC combat pairings.
    pub melee_target: Option<EntityId>,

    /// Initial action set from level data (beam-me `actionInitial`).
    /// Evaluated by action initialization to set the PC's starting
    /// state.
    pub initial_action: u32,

    /// Forbidden hero expression list (expression_id, forbid_timer).
    /// Each entry counts down each frame and is removed at 0, preventing
    /// the same expression from repeating too quickly.
    pub forbidden_expressions: Vec<(u16, u16)>,

    /// Last `combat_anim` id observed by the speech-trigger tick — used
    /// to detect the START of a new animation and the DONE transition
    /// (anim cleared) for a remark played after an action.
    pub prev_combat_anim_id: u32,
    pub prev_combat_anim_ot: Option<crate::order::OrderType>,
}

impl Default for PcData {
    fn default() -> Self {
        Self {
            life_points: crate::pc_status::LIFEPOINTS_PC,
            immortal: false,
            robin: false,
            already_selected: false,
            list_index: 0,
            cached_camp: Camp::Royalists,
            campaign_description_index: None,
            playable: true,
            command_interface: CommandInterface::HeroActions,
            mission_role: MissionRole::PlayerParty,
            combat_stance: CombatStance::Aggressive,
            ai: None,
            interface_hidden: false,
            current_action: Action::default(),
            saved_action: Action::default(),
            disabled_actions: Vec::new(),
            disabled_actions_temp: Vec::new(),
            ammo: PcAmmoData::default(),
            quick_action_types: vec![QuickAction::None; 3],
            quick_action_sequences: vec![None, None, None],
            quick_seek_sequences: vec![None, None, None],
            quick_action_special_counts: vec![0; 3],
            quick_action_buttons: vec![0; 3],
            quick_action_interactors: vec![None; 3],
            titbits: vec![None; 3],
            portrait: PcPortraitState::default(),
            head_seen: false,
            belt_seen: false,
            feet_seen: false,
            position_before_teleport: MapPoint::default(),
            teleport_counter: 0,
            max_teleport_counter: 0,
            fried_psykokwack: false,
            carried: None,
            carried_posture: Posture::Undefined as u32,
            shield_danger_point: WorldPoint3D::default(),
            shield_danger_point_layer: 0,
            shield_protected: None,
            shield_protector: None,
            guard: None,
            time_till_reinforcement: 0xFFFF_FFFF,
            work_icon: WorkIcon::default(),
            last_ammo_dropping_position: MapPoint::default(),
            last_dropped_ammo: None,
            update_last_dropped_ammo: false,
            last_dropping_direction: 0,
            profile_index: CharacterProfileIdx(0),
            kind: None,
            has_lockpick: false,
            has_climb: false,
            has_jump: false,
            beam_me_index: -1,
            trumpet_enabled: false,
            melee_target: None,
            initial_action: 0,
            forbidden_expressions: Vec::new(),
            prev_combat_anim_id: 0,
            prev_combat_anim_ot: None,
        }
    }
}

impl PcData {
    /// Apply the original game's player-character playability state change.
    ///
    /// In retail missions, making a PRIS rescue PC playable is also the
    /// boundary where that scripted prisoner becomes an ordinary party hero.
    /// Those facts were implicit in the original game; keep the
    /// transition together now that command surface and mission role are
    /// represented independently.
    pub fn set_playable(&mut self, playable: bool) {
        self.playable = playable;
        if playable && self.mission_role == MissionRole::RescueTarget {
            self.mission_role = MissionRole::PlayerParty;
            self.command_interface = CommandInterface::HeroActions;
            self.combat_stance = CombatStance::Aggressive;
        }
    }

    pub fn live_carried_posture(&self) -> Posture {
        Posture::try_from(self.carried_posture).unwrap_or_else(|_| {
            panic!(
                "live carried_posture contains invalid original-game enum word {}",
                self.carried_posture
            )
        })
    }

    pub fn set_live_carried_posture(&mut self, posture: Posture) {
        self.carried_posture = posture as u32;
    }

    pub fn movement_auth_from_profile(profile: &CharacterProfile) -> (bool, bool, bool) {
        (
            profile.has_contextual_action(Action::Lockpick),
            profile.has_contextual_action(Action::Climb),
            profile.has_contextual_action(Action::Jump),
        )
    }
}

impl PcData {
    /// Unconditionally save the current action and clear it; then,
    /// **only if `playable`**, mark every action temp-disabled. The
    /// widget messaging side-effect is omitted — the HUD reads
    /// `disabled_actions_temp` directly each frame.
    pub fn disable_all_actions_temp(&mut self) {
        self.saved_action = self.current_action;
        self.current_action = Action::default();
        if self.playable {
            for slot in self.disabled_actions_temp.iter_mut() {
                *slot = true;
            }
        }
    }

    /// Gated on `!is_swordfighting && playable`. Inside the guard each
    /// temp-disabled slot is conditionally cleared, and if any cleared
    /// slot's authored action matches `saved_action` (and the permanent mask
    /// is also clear), the saved action is returned for the engine to forward
    /// through `MSG_SELECT_ACTION` semantics.
    /// The widget messaging side-effect is omitted — the HUD reads
    /// state directly.
    ///
    /// `is_swordfighting` is provided by the caller because the
    /// authoritative check (`HumanData::opponents.is_empty()`) lives on
    /// the human layer and we don't take the whole `Entity` here.
    pub fn enable_all_actions_temp(
        &mut self,
        is_swordfighting: bool,
        actions: &[Action; crate::profiles::NUMBER_OF_PC_ACTIONS],
    ) -> Option<Action> {
        if is_swordfighting || !self.playable {
            return None;
        }
        let mut restore_action = None;
        for (idx, slot) in self.disabled_actions_temp.iter_mut().enumerate() {
            if *slot {
                *slot = false;
                let permanent_disabled = self.disabled_actions.get(idx).copied().unwrap_or(false);
                if !permanent_disabled
                    && actions.get(idx).copied() == Some(self.saved_action)
                    && restore_action.is_none()
                {
                    restore_action = Some(self.saved_action);
                }
            }
        }
        restore_action
    }
}

/// A detectable entity tracked by NPC vision.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Detectable {
    pub element: Option<EntityId>,
    pub detectable_type: DetectableType,
    pub seen_last_frame: bool,
    pub heard_last_frame: bool,
    pub seen_now: bool,
    pub shadow_seen_now: bool,
    pub shadow_seen_last_frame: bool,
    pub last_visibility: f32,
}

impl Default for Detectable {
    fn default() -> Self {
        Self {
            element: None,
            detectable_type: DetectableType::None,
            seen_last_frame: false,
            heard_last_frame: false,
            seen_now: false,
            shadow_seen_now: false,
            shadow_seen_last_frame: false,
            last_visibility: 0.0,
        }
    }
}

/// AI brain enum.  Each NPC owns one of these; soldiers get
/// [`EnemyAi`], civilians get [`FriendlyAi`].
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AiBrain {
    #[default]
    None,
    Enemy(Box<EnemyAi>),
    Friendly(Box<FriendlyAi>),
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum PersistedAiBrain {
    None,
    Enemy(Box<crate::ai::persisted::PersistedEnemyAi>),
    Friendly(Box<crate::ai::persisted::PersistedFriendlyAi>),
}

impl PersistedAiBrain {
    pub(crate) fn capture(value: &AiBrain) -> Self {
        match value {
            AiBrain::None => Self::None,
            AiBrain::Enemy(value) => Self::Enemy(Box::new(
                crate::ai::persisted::PersistedEnemyAi::capture(value),
            )),
            AiBrain::Friendly(value) => Self::Friendly(Box::new(
                crate::ai::persisted::PersistedFriendlyAi::capture(value),
            )),
        }
    }
    pub(crate) fn into_runtime(self) -> AiBrain {
        match self {
            Self::None => AiBrain::None,
            Self::Enemy(value) => AiBrain::Enemy(Box::new(value.into_runtime())),
            Self::Friendly(value) => AiBrain::Friendly(Box::new(value.into_runtime())),
        }
    }
}

impl AiBrain {
    /// Access the base `AiController` (common to both enemy and friendly).
    pub fn base(&self) -> Option<&AiController> {
        match self {
            Self::None => None,
            Self::Enemy(e) => Some(&e.base),
            Self::Friendly(f) => Some(&f.base),
        }
    }

    /// Mutable access to the base `AiController`.
    pub fn base_mut(&mut self) -> Option<&mut AiController> {
        match self {
            Self::None => None,
            Self::Enemy(e) => Some(&mut e.base),
            Self::Friendly(f) => Some(&mut f.base),
        }
    }

    /// Access the enemy AI subclass, if this is a soldier.
    pub fn enemy(&self) -> Option<&EnemyAi> {
        match self {
            Self::Enemy(e) => Some(e),
            _ => None,
        }
    }

    /// Mutable access to the enemy AI subclass.
    pub fn enemy_mut(&mut self) -> Option<&mut EnemyAi> {
        match self {
            Self::Enemy(e) => Some(e),
            _ => None,
        }
    }

    /// Access the friendly AI subclass, if this is a civilian.
    pub fn friendly(&self) -> Option<&FriendlyAi> {
        match self {
            Self::Friendly(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable access to the friendly AI subclass.
    pub fn friendly_mut(&mut self) -> Option<&mut FriendlyAi> {
        match self {
            Self::Friendly(f) => Some(f),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Per-actor state owned by the NPC AI runtime.
///
/// This is separate from [`NpcData`] so an AI-controlled hero can run the same
/// perception and battle-decision machinery without masquerading as an NPC or
/// carrying a second, potentially divergent copy of its body health.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiActorData {
    /// Persistent NPC-only construction ordinal.
    ///
    /// The original-game NPC actor assigns this from
    /// `guwNPCRegisterNumber` and uses it to stagger periodic work. It is
    /// distinct from both the entity-table slot and element creation order
    /// because non-NPC elements do not increment the counter.
    pub register_number: u16,
    pub number_of_arrows: u16,

    pub direction_old: i16,
    pub initial_view_direction: MapVec,
    pub initial_position_x: f32,
    pub initial_position_y: f32,
    pub initial_position_sector: Option<crate::position_interface::SectorHandle>,
    pub initial_position_level: u16,

    pub inform_my_friends: bool,
    pub money: u32,
    pub wasp_victim: bool,

    pub old_cover_noise_deafness: u16,
    pub old_cover_noise_deafness_frame_counter: u32,

    /// Frames spent stuck on an outdoor ladder while idle.  Bumped
    /// each tick by `tick_npc_stuck_on_ladder_for_npc` when the NPC is on a
    /// ladder in a non-building sector with command `Wait`/`MoveWaiting`
    /// and not script-locked; reset otherwise.  After 25 frames the
    /// engine forces a return to duty so NPCs that hang on outdoor
    /// ladders can self-recover.
    pub stuck_on_ladder_emergency_counter: u16,

    /// Exact dialog-scroll entity attached by Original's
    /// NPC attached scroll.
    ///
    /// Keeping the identity (rather than only a boolean) matters when a
    /// loaded NPC is clicked: the interaction must open the same scroll
    /// object which was serialized in the save.
    pub attached_scroll: Option<EntityId>,

    /// Original-game serialized body-visitor continuation counter.
    pub body_visitors: u16,

    /// Original-game serialized special script latch. The original game stores
    /// this for NPCs even though its AI tick never reads it.
    pub fried_pikachu: bool,

    /// One detection list per [`DetectableType`] (indexed 0..COUNT).
    pub detectable_lists: Vec<Vec<Detectable>>,
    pub detection_suspects: [u16; DetectableType::COUNT],
    pub maximal_detection_suspect: u16,
    pub worst_detected_type: DetectableType,

    pub has_given_money_to_beggar: bool,

    pub custom_values: [i32; NpcCustomValue::COUNT],

    pub display_double_status_bar: bool,

    // -- Cross-module reference: AI controller --
    /// The NPC's AI brain — either an enemy AI or a civilian AI.
    pub ai_brain: AiBrain,

    /// `true` once this NPC has spotted a hostile and is actively pursuing
    /// or attacking.  Kept in sync with `ai_state == Attacking` by
    /// `EngineInner::tick_enemy_ai`.  Exists as a cheap flag so combat checks
    /// don't need to crack open the full AI controller.
    pub alerted: bool,

    /// Real view radius (map units).  Initialized from
    /// the engine's standard-view-radius helper at level load —
    /// day/night dependent — and subsequently mutated by the AI
    /// (alertness,
    /// drunk cone iterator, lean-out, etc).  For now we only track the
    /// base value; the per-frame mutation logic in view refresh is
    /// not yet implemented.
    pub view_radius: u16,

    /// Eye / view status — controls whether the NPC can see at all.
    /// When set to `EyeStatus::Closed` or
    /// `EyeStatus::DieOrGetUnconscious` the vision pipeline returns
    /// 0.0 visibility.
    pub eye_status: EyeStatus,

    /// Live half-aperture (radians) used for NPC vision geometry.
    /// The *real* vision cone is built with this value; it starts at
    /// `NORMAL_HALF_APERTURE` but is mutated at runtime by alert
    /// state, drunk-cone iterator, `ViewconeGrow` status, lean-out
    /// posture, and the forest-level Royalist 180° special case.
    ///
    /// **Mutation is not yet fully implemented.** The value stays at the
    /// initial `NORMAL_HALF_APERTURE` until view-refresh logic
    /// lands.  The view cone overlay and AI vision code read this
    /// field so the port is ready to pick up the real values once
    /// mutation is wired.
    pub half_aperture: f32,

    /// "Real" half-aperture after all modifiers (stare, drunk, etc.).
    /// Updated by `ai_vision::refresh_view` each frame.
    pub real_half_aperture: f32,

    // -- View state --
    // Populated by `ai_vision::refresh_view` each frame.
    /// View angle offset from body direction (radians).  Head turns
    /// (look-left/right) and stare/follow rotate the view cone
    /// relative to the body.
    pub view_angle: f32,

    /// Per-frame angle step for view transitions (default π/16).
    pub view_angle_step: f32,

    /// Set when the body direction or eye status changes; cleared
    /// when the view angle reaches its goal.
    pub view_transition: bool,

    /// Maximum angular deviation from body direction during head turns.
    pub view_half_angle_range: f32,

    /// Serialized view-cone angle oscillator continuation state.
    pub view_angle_iterator: f32,
    pub view_angle_iterator_step: f32,

    /// Base view radius before modifiers (longrange, drunk, rider).
    /// The final computed radius is stored in `view_radius`.
    pub view_radius_base: u16,

    /// Target radius for grow / death-shrink animations.
    pub view_radius_goal: u16,

    /// Accelerating step for the death-shrink radius animation.
    pub view_radius_step: u16,

    /// Alpha intensity for the view cone overlay (0-255).
    pub view_alpha_start: u16,

    /// Long-range radius multiplier (default 1.0).
    pub view_longrange_radius_factor: f32,

    /// Serialized aperture-transition continuation state. The transition
    /// block is disabled in the shipped original game, but these members
    /// remain part of the authoritative save image.
    pub view_half_aperture_cosine: f32,
    pub view_future_half_aperture: f32,
    pub view_half_aperture_step: f32,
    pub view_half_aperture_changes: bool,

    /// Serialized "crazy" cone oscillator continuation state.
    pub view_crazy_angle_iterator: f32,
    pub view_crazy_angle_iterator_step: f32,
    pub view_crazy_color_iterator: u8,
    pub view_crazy_half_angle_range: f32,

    /// Computed view direction (body direction rotated by `view_angle`).
    /// Updated by `refresh_view` each frame.
    pub view_direction: [f32; 2],

    /// Serialized cone boundary vectors. Original consumes these in
    /// visibility tests until view refresh computes the next pair.
    pub view_left_side: [f32; 2],
    pub view_right_side: [f32; 2],

    /// Whether the NPC is currently leaning out.
    pub view_lean_out: bool,

    /// Four phase iterators for drunken vision cone wobble.
    pub drunken_cone_iterators: [f32; 4],

    /// Serialized radius-reduction and sniper view flags.
    pub view_radius_reduction_permil: u16,
    pub view_sniper: bool,

    /// Point the NPC is staring at (for `EyeStatus::Stare`).
    /// Original-game stare point: world-ground `(x, y)`, not
    /// projected map coordinates. Position focus and eye-following
    /// populate this from a 3D point's X/Y components.
    pub stare_point: GroundPoint,

    /// Entity the view cone follows (for `EyeStatus::Follow`).
    pub follow_target: Option<EntityId>,
}

/// View / eye status enum.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u8)]
pub enum EyeStatus {
    Closed = 0,
    #[default]
    LookForward,
    LookToTheLeft,
    LookToTheRight,
    LookDownwards,
    DieOrGetUnconscious,
    Follow,
    Stare,
    ViewconeGrow,
}

impl EyeStatus {
    /// `true` when the NPC's eyes are non-functional and visibility
    /// must short-circuit to 0.
    #[inline]
    pub fn is_blind(self) -> bool {
        matches!(self, Self::Closed | Self::DieOrGetUnconscious)
    }
}

impl Default for AiActorData {
    fn default() -> Self {
        Self {
            register_number: 0,
            // Seed `MAX_NPC_ARROWS` for every NPC unconditionally —
            // civilians, friendlies, hostile soldiers — even those
            // who never use a bow.  Arrows are only consumed when a
            // bow shot resolves, so the spare quiver on non-archers
            // is harmless. Without this seed, bow-carrying enemy
            // soldiers would spawn with 0 arrows and fire nothing
            // until the `FleeingRunForArrowReserves` refill path
            // triggers.
            number_of_arrows: crate::parameters_ai::MAX_NPC_ARROWS as u16,
            direction_old: 0,
            initial_view_direction: MapVec::default(),
            initial_position_x: 0.0,
            initial_position_y: 0.0,
            initial_position_sector: None,
            initial_position_level: 0,
            inform_my_friends: false,
            money: 0,
            wasp_victim: false,
            old_cover_noise_deafness: 0,
            old_cover_noise_deafness_frame_counter: 0,
            stuck_on_ladder_emergency_counter: 0,
            attached_scroll: None,
            body_visitors: 0,
            fried_pikachu: false,
            detectable_lists: vec![Vec::new(); DetectableType::COUNT],
            detection_suspects: [0; DetectableType::COUNT],
            maximal_detection_suspect: 0,
            worst_detected_type: DetectableType::None,
            has_given_money_to_beggar: false,
            custom_values: [0; NpcCustomValue::COUNT],
            display_double_status_bar: false,
            ai_brain: AiBrain::None,
            alerted: false,
            // The engine overwrites this with the correct day/night
            // view radius during level-load initialization,
            // but 400 is a safe fallback if nothing wires it up.
            view_radius: 400,
            eye_status: EyeStatus::LookForward,
            // `NORMAL_HALF_APERTURE = 0.5 rad` (~28.6°). This is the
            // initial value before per-alert mutation kicks in.
            half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            view_angle: 0.0,
            view_angle_step: crate::ai_vision::NORMAL_ANGLE_STEP,
            view_transition: false,
            view_half_angle_range: crate::ai_vision::NORMAL_HALF_ANGLE_RANGE,
            view_angle_iterator: 0.0,
            view_angle_iterator_step: crate::ai_vision::NORMAL_ANGLE_ITERATOR_STEP,
            view_radius_base: 400,
            view_radius_goal: 400,
            // Original-game NPC initialization sets the radius step to 10. The
            // DieOrGetUnconscious view-cone collapse consumes this value on
            // its first view refresh and then accelerates it by 5 each frame.
            view_radius_step: 10,
            view_alpha_start: crate::ai_vision::ALPHA_START,
            view_longrange_radius_factor: 1.0,
            view_half_aperture_cosine: crate::ai_vision::NORMAL_HALF_APERTURE.cos(),
            view_future_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            view_half_aperture_step: crate::parameters_ai::HALF_APERTURE_STEP,
            view_half_aperture_changes: false,
            view_crazy_angle_iterator: 0.0,
            view_crazy_angle_iterator_step: crate::parameters_ai::CRAZY_TREMBLE_ITERATOR_STEP,
            view_crazy_color_iterator: 0,
            view_crazy_half_angle_range: crate::parameters_ai::CRAZY_TREMBLE_RANGE,
            view_direction: [1.0, 0.0],
            view_left_side: [1.0, 0.0],
            view_right_side: [1.0, 0.0],
            view_lean_out: false,
            drunken_cone_iterators: [0.0; 4],
            view_radius_reduction_permil: 1000,
            view_sniper: false,
            stare_point: GroundPoint::new(0.0, 0.0),
            follow_target: None,
        }
    }
}

impl AiActorData {
    /// Original-game NPC detectable deletion removes only the first
    /// matching entry and report whether one was found. Release recordings can
    /// contain duplicates because several original-game insertion paths only
    /// guard uniqueness with a debug assertion.
    pub fn delete_detectable(
        &mut self,
        element: EntityId,
        detectable_type: DetectableType,
    ) -> bool {
        let index = detectable_type as usize;
        let list = self.detectable_lists.get_mut(index).unwrap_or_else(|| {
            panic!(
                "NPC has no {:?} detectable list at index {index}",
                detectable_type
            )
        });
        let Some(position) = list
            .iter()
            .position(|detectable| detectable.element == Some(element))
        else {
            return false;
        };
        list.remove(position);
        true
    }

    /// Current AI top-level state, read from the owning [`AiController`]
    /// (single source of truth).  NPCs without an AI brain report
    /// [`AiTopState::Default`], matching the pre-consolidation default
    /// value of the removed stored field.
    pub fn ai_state(&self) -> AiTopState {
        self.ai_brain
            .base()
            .map(|b| b.current_state)
            .unwrap_or(AiTopState::Default)
    }

    /// Current AI substate, read from the owning [`AiController`]
    /// (single source of truth).  NPCs without an AI brain report
    /// [`AiSubstate::DefaultOnPost`], matching the pre-consolidation
    /// default value of the removed stored field.
    pub fn ai_substate(&self) -> AiSubstate {
        self.ai_brain
            .base()
            .map(|b| b.current_substate)
            .unwrap_or(AiSubstate::DefaultOnPost)
    }

    /// Returns the current cover-noise deafness after applying decay.
    /// The engine should call this each frame via the hearing path.
    ///
    /// `cover_volume` is the max of every active sound source's
    /// covering-volume-at-position at the NPC's position.  The caller
    /// supplies it because `NpcData` has no access to the engine's
    /// `SoundSourceManager`; pass `0` when no sound sources should
    /// mask hearing.
    pub fn get_deafness(&mut self, current_frame: u32, cover_volume: u16) -> u16 {
        use crate::parameters_ai;

        // Same-frame short-circuit.  Only fires when the
        // function has already been called this frame; the
        // `cover_volume` argument is irrelevant here because the prior
        // call already folded the per-frame covering volume into the
        // stored value.
        if self.old_cover_noise_deafness_frame_counter == current_frame {
            return self.old_cover_noise_deafness;
        }

        // Catch-up decay loop.  Runs until the counter catches up OR
        // the deafness reaches zero.  No iteration cap — the decay
        // rate guarantees a bounded number of steps before deafness
        // reaches zero (slow decay alone needs at most
        // ceil(300 / AI_DEAFNESS_MINUS) iterations to bottom out).
        while self.old_cover_noise_deafness_frame_counter < current_frame
            && self.old_cover_noise_deafness > 0
        {
            // Stepped fast decay above 300.  The integer division
            // `(deaf / RADIUS)` evaluates BEFORE the multiplication,
            // giving a stepped reduction:
            //   deaf in [300, 599]  → subtract 50 * 1
            //   deaf in [600, 899]  → subtract 50 * 2
            //   …
            // Slow decay below 300 subtracts a flat `AI_DEAFNESS_MINUS`.
            if self.old_cover_noise_deafness > parameters_ai::AI_QUICK_DEAFNESS_RADIUS as u16 {
                let fast = (parameters_ai::AI_QUICK_DEAFNESS_MINUS as u32
                    * (self.old_cover_noise_deafness as u32
                        / parameters_ai::AI_QUICK_DEAFNESS_RADIUS as u32))
                    as u16;
                self.old_cover_noise_deafness = self.old_cover_noise_deafness.saturating_sub(fast);
            } else {
                self.old_cover_noise_deafness = self
                    .old_cover_noise_deafness
                    .saturating_sub(parameters_ai::AI_DEAFNESS_MINUS as u16);
            }
            self.old_cover_noise_deafness_frame_counter = self
                .old_cover_noise_deafness_frame_counter
                .saturating_add(1);
        }
        // If we exited the loop because deafness hit zero before the
        // counter caught up, snap the counter forward so the same-frame
        // short-circuit at the top of the function fires correctly on
        // subsequent calls this frame.
        self.old_cover_noise_deafness_frame_counter = current_frame;

        // Take the max of current deafness and the covering volume
        // from active sound sources at this position.  The caller
        // pre-computes `cover_volume` because `NpcData` lacks access
        // to the `SoundSourceManager`.
        if cover_volume > self.old_cover_noise_deafness {
            self.old_cover_noise_deafness = cover_volume;
        }

        self.old_cover_noise_deafness
    }

    /// Zeroes every per-detectable suspect accumulator and the cached
    /// worst-threat summary.  Called when the NPC transitions into
    /// unconsciousness so the pre-knockout hostility tint / blip color
    /// doesn't leak to wake-up.
    pub fn clear_all_suspects(&mut self) {
        for slot in self.detection_suspects.iter_mut() {
            *slot = 0;
        }
        self.maximal_detection_suspect = 0;
        self.worst_detected_type = DetectableType::None;
    }
}

/// Body state shared by soldier and civilian entities plus their AI runtime.
///
/// `Deref` preserves the existing `npc.ai_brain`/vision-field API while making
/// the ownership boundary explicit for actors which are not NPC bodies.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NpcData {
    pub life_points: i16,
    #[serde(flatten)]
    pub ai: AiActorData,
}

impl Default for NpcData {
    fn default() -> Self {
        Self {
            life_points: crate::pc_status::LIFEPOINTS_PC,
            ai: AiActorData::default(),
        }
    }
}

impl std::ops::Deref for NpcData {
    type Target = AiActorData;

    fn deref(&self) -> &Self::Target {
        &self.ai
    }
}

impl std::ops::DerefMut for NpcData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ai
    }
}

/// Soldier-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoldierData {
    pub apple_smell: u32,
    /// References soldier profile data.
    pub soldier_profile_index: SoldierProfileIdx,
    /// Cached from profile at creation time.
    pub cached_max_life_points: i16,
    /// Cached from profile at creation time.
    pub cached_camp: Camp,
    /// Whether this soldier is mounted on a horse.
    pub rider: bool,
    /// Optional player-facing tactical order surface. EnemyAi remains the
    /// decision policy even when this is enabled.
    #[serde(default)]
    pub command_interface: CommandInterface,
    /// Mission bookkeeping role, independent from allegiance.
    #[serde(default)]
    pub mission_role: MissionRole,
    /// Default stance before/without an explicit tactical order.
    #[serde(default)]
    pub combat_stance: CombatStance,
}

/// Civilian-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CivilianData {
    pub current_scroll_set: u32,
    /// References civilian profile data.
    pub civilian_profile_index: CivilianProfileIdx,
    /// Cached from profile at creation time.
    pub cached_camp: Camp,
    /// Cached civilian type (Beggar/Child/Vip/Standard) from profile
    /// at load time.
    pub cached_civilian_type: crate::profiles::CivilianType,
    /// Per-scroll-set scroll IDs for beggar civilians (10 sets).
    /// `None` for non-beggar civilians.
    pub beggar_scroll_sets: Option<Vec<Vec<u16>>>,
}

/// FX-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FxData {
    pub restore_background: bool,
    pub force_display: bool,
    pub animation: Animation,
    /// Masking polyline for display order interleaving.  FX entities
    /// with a non-empty polyline participate in the animation merge
    /// pass of display sorting (characters can walk behind them).
    pub display_polyline: Vec<MapPoint>,
    /// If this FX entity is a patch's animation element, the patch
    /// index (into the canonical interactable patch table). Used by the animation tick
    /// to apply reversed playback and detect transition completion.
    pub patch_index: Option<crate::patch::PatchIndex>,
    /// Index of the owning mission mobile element for its masked child
    /// sprite. `None` for ordinary proto/patch FX.
    #[serde(default)]
    pub mobile_index: Option<u16>,
    /// Original-game masked-FX animation speed. Mobile carts update this
    /// to `1 / movement_speed`; ordinary FX retain `1.0`.
    #[serde(default = "default_fx_animation_speed")]
    pub animation_speed: f32,
    /// Rendering properties (`Blocky` vs `NeedShadow`) selected from
    /// the blit-type byte at level load.  `NeedShadow` selects the
    /// alpha-keyed drawing path (sprite gets the global shadow tint
    /// composited on); `Blocky` selects plain `Blit` with no shadow
    /// compositing.
    pub rendering_properties: RenderingProperties,
}

const fn default_fx_animation_speed() -> f32 {
    1.0
}

impl Default for FxData {
    fn default() -> Self {
        Self {
            restore_background: false,
            force_display: false,
            animation: Animation::default(),
            display_polyline: Vec::new(),
            patch_index: None,
            mobile_index: None,
            animation_speed: 1.0,
            rendering_properties: RenderingProperties::default(),
        }
    }
}

/// Target-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TargetData {
    pub animation: Animation,
    /// Raw frame-progression ordinal from level data.
    pub progression: u32,
    pub linked_fx: Vec<EntityId>,
    /// Action filter flags determining which PC actions interact with this target.
    /// See `TargetFilter` bitflags.
    pub action_filter: TargetFilter,
    /// Action point — the position the PC walks to when interacting
    /// with this target.
    pub action_position: MapPoint,
    /// Action point sector.
    pub action_sector: u16,
    /// Action point layer.
    pub action_layer: u16,
    /// Optional Z height for elevated targets (negative = no Z).
    pub position_z: i16,
    /// Sprite filename (.rhs file).
    pub sprite_filename: String,
    /// Sprite profile name within the .rhs.
    pub sprite_profile_name: String,
    /// Masking polyline used for blit clipping.
    pub display_polyline: Vec<MapPoint>,
    /// Rendering properties (`Blocky` vs `NeedShadow`) selected from the
    /// blit type byte in the level data.
    pub rendering_properties: RenderingProperties,
    /// Script class name driving this target's per-instance script
    /// (`IElementTargetScript` implementation).  Loaded from the proto
    /// stream's target script-class field.  Each target carries its
    /// own VM, with named functions like `ActivatedByListenable`,
    /// `ActivatedByApple`, etc.  Empty string means "no script
    /// attached" (the dispatcher skips such targets).
    pub script_class: String,
}

impl Default for TargetData {
    fn default() -> Self {
        Self {
            // Initial animation is `WaitingUpright`.
            // `Animation::default()` would otherwise give ordinal 0
            // (`WaitingUprightBored`), which is a different pose.
            animation: Animation::WaitingUpright,
            progression: 0,
            linked_fx: Vec::new(),
            action_filter: TargetFilter::empty(),
            action_position: MapPoint::default(),
            action_sector: 0,
            action_layer: 0,
            position_z: -1,
            sprite_filename: String::new(),
            sprite_profile_name: String::new(),
            display_polyline: Vec::new(),
            rendering_properties: RenderingProperties::Blocky,
            script_class: String::new(),
        }
    }
}

/// Object-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ObjectData {
    pub associated_action: Action,
    pub terminate: bool,
    pub quantity: u16,
    pub object_type: ObjectType,
    pub animation: Animation,
    pub reference: Option<EntityId>,
    pub belongs_to_beggar: bool,
    pub taken: bool,
}

impl Default for ObjectData {
    fn default() -> Self {
        Self {
            associated_action: Action::default(),
            terminate: false,
            quantity: 1,
            object_type: ObjectType::default(),
            // Initial animation is `WaitingUpright`, not the enum-zero
            // `WaitingUprightBored`.
            animation: Animation::WaitingUpright,
            reference: None,
            belongs_to_beggar: false,
            taken: false,
        }
    }
}

/// A single waypoint on a precomputed ballistic trajectory.
///
/// The projectile moves linearly from its current position to
/// `position` over `time` frames before popping the next point from
/// the trajectory list.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TrajectoryPoint {
    pub position: WorldPoint3D,
    /// Number of frames to reach this point from the previous position.
    pub time: u16,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TrajectoryPointRuntime {
    pub bounce: bool,
    pub material: u32,
}

/// Projectile-level data.
#[derive(
    Default,
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProjectileData {
    /// 3D launch point.  Read by the trajectory-arc debug overlay
    /// (`game_render::draw_trajectories`) to render from the launch
    /// origin rather than the current frame.
    pub start: WorldPoint3D,
    pub end: WorldPoint3D,
    /// X of the launch point.  Read by `EventGetArrow` (arrow hit) and
    /// `EventApple` (apple hit) so the AI stimulus anchors at the
    /// shooter's *original* position rather than the impact site.
    pub start_of_trajectory_x: f32,
    /// Y of the launch point.  Paired with `start_of_trajectory_x`.
    pub start_of_trajectory_y: f32,
    pub shooter: Option<EntityId>,
    pub frame_count: u16,
    pub flying: bool,
    /// One-shot latch for a ground-targeted stone. The first terminal impact
    /// broadcasts [`crate::ai::NoiseType::Distraction`] and clears this bit.
    #[serde(default)]
    pub noise_distraction: bool,
    /// Original-game water/hole landing latch.
    #[serde(default)]
    pub dive: bool,
    /// Bloodseeker-oil straight-flight latch.
    #[serde(default)]
    pub magic_bullet: bool,
    pub disappear: bool,
    /// Precomputed trajectory waypoints.  `tick_arrows` pops points
    /// from the front and interpolates position toward each one over
    /// `time` frames.
    pub trajectory: Vec<TrajectoryPoint>,
    /// Serialized metadata parallel to `trajectory`. Empty for a freshly
    /// generated trajectory until its collision metadata is materialized.
    #[serde(default)]
    pub trajectory_runtime: Vec<TrajectoryPointRuntime>,
    /// Constructor-only latch: generated trajectory runtime still needs its
    /// exact collision waypoint materialized before the first update.
    /// Cleared before publication by the purse/coin constructor boundary.
    #[serde(default)]
    pub terminal_material_pending: bool,
    /// Exact raw collision waypoint retained by trajectory calculation until the
    /// pre-publication update can determine its material.
    #[serde(default)]
    pub terminal_material_impact_index: Option<u16>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_sector: Option<u16>,
    /// Exact arena half of the original game's trajectory-start sector.
    /// The public number above remains for backward-compatible serialized
    /// state, but AI projectile-hit callbacks must copy the complete sector
    /// pointer identity into their stimulus position.
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub trajectory_origin_layer: Option<crate::position_interface::Layer>,
    /// Per-frame position delta for the current trajectory segment.
    /// Recomputed each time a new waypoint is popped.
    pub velocity_increment: WorldVec3D,
    /// Original-game flight direction, sampled from the initial trajectory
    /// velocity. This is gameplay state used to orient hit actors; it is
    /// deliberately separate from the projectile element's sprite facing.
    #[serde(default)]
    pub flight_direction: u16,
    /// Optional explicit start point for the next collision segment.
    /// Used when a projectile is advanced once before entering the
    /// engine list, matching the original game's spawn-time advance.
    pub launch_segment_start: Option<WorldPoint3D>,
    /// Frames remaining in the current trajectory segment.
    /// When this reaches 0, the next `TrajectoryPoint` is popped.
    pub trajectory_frame_count: u16,
    /// Precomputed damage for this projectile.  Set at spawn time from the
    /// shooter's bow profile via `BowState::get_damage()`.  Applied on hit
    /// by `apply_arrow_hit`.
    pub damage: u16,
    /// True when the arrow has been deflected (by a shield or target) and
    /// is falling to the ground.  Falling arrows skip shield and victim
    /// collision checks.
    pub falling: bool,
    /// Last non-falling horizontal orientation computed by
    /// arrow refresh. The original game serializes this cache and reuses it
    /// when the trajectory has become empty.
    #[serde(default)]
    pub last_orientation_sector: u16,
    /// Last non-falling vertical orientation, in degrees in `[-60, 60]`.
    #[serde(default)]
    pub last_orientation_azimuth: i16,
    /// Sector (0..15) used by a falling arrow's visual rotation. Cycled by
    /// the deferred arrow-refresh boundary, not by the element tick.
    pub falling_direction: u16,
    /// Arrow-only serialized leaf state. `arrow_bow_profile` distinguishes a
    /// null bow (`None`) from a present default bow (`Some(None)`).
    #[serde(default)]
    pub arrow_bow_profile: Option<Option<u32>>,
    #[serde(default)]
    pub arrow_flat_shot: bool,
    #[serde(default)]
    pub arrow_play_impact: bool,
    /// Purse / coin back-pointers — populated for `ObjectType::Purse`
    /// and `ObjectType::Coin` projectiles, default for everything else.
    /// See [`PurseData`].
    pub purse: PurseData,
    /// Wasp-nest / wasp state — populated for
    /// `ObjectType::BonusWaspNest` / `ObjectType::WaspNest` parents and
    /// each `ObjectType::Wasp` child, default for everything else.
    /// See [`WaspData`].
    pub wasp: WaspData,
}

/// Net-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NetData {
    pub victims: Vec<EntityId>,
    /// Pre-landing countdown (frames). Set at spawn time to
    /// `total_trajectory_frames - 15`.  Decrements during flight;
    /// when it reaches 0 the net switches its sprite animation to
    /// `NetUnfolding` (or `NetUnfoldingCrumpled`).
    pub time_till_unfolding: u32,
    pub crumpled: bool,
    pub was_flying: bool,
    /// IDs of the (up to two) `RepulsivePoint`s registered on
    /// `AiGlobalState` while the net sits on the ground.  Cleared on
    /// despawn.
    pub repulsive_point_ids: Vec<i32>,
    /// True once the post-landing animation transition has fired
    /// (NetUnfolding → ObjectLying / NetMoving, or
    /// NetUnfoldingCrumpled → NetLyingCrumpled). Prevents repeating
    /// the transition each frame.
    pub landed_animation_resolved: bool,
}

/// Purse / coin back-pointers.
///
/// A single struct serves both roles since purses and coins are
/// sibling kinds of projectile: purses populate `child_coins` /
/// `number_of_coins` and leave `source_purse` empty; coins populate
/// `source_purse` and leave the other fields empty.  Lives on
/// [`ProjectileData`] so it travels with the existing
/// `Entity::Projectile(ElementProjectile)` payload — no extra entity
/// variant needed.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PurseData {
    /// On a coin: handle of the purse it was ejected from.  On a
    /// purse: always `None`.
    pub source_purse: Option<EntityId>,
    /// On a purse: list of child coins spawned by obstacle impact. On
    /// a coin: empty.
    pub child_coins: Vec<EntityId>,
    /// On a purse: number of coins still owed (decremented each burst).
    /// Initialised to `inventory::COINS_PER_PURSE` at spawn.  On a
    /// coin: always 0.
    pub number_of_coins: u16,
    /// On a purse: true once obstacle impact has occurred and child coins
    /// are in flight. Used by the update to know when to switch to
    /// the "drain children, then despawn" idle phase.
    pub burst: bool,
    /// On a coin: layer the coin should snap to on landing.  Stored
    /// at spawn so obstacle impact can re-key the coin onto its goal
    /// layer. On a purse: `None` because the field is not applicable.
    #[serde(deserialize_with = "Option::deserialize")]
    pub layer_goal: Option<crate::position_interface::Layer>,
    /// On a coin: sector the coin should snap to on landing (None
    /// when the scatter target wasn't resolved against a known
    /// sector).
    #[serde(deserialize_with = "Option::deserialize")]
    pub sector_goal: Option<crate::position_interface::SectorHandle>,
}

/// Wasp-nest / wasp shared fields.  Lives on [`ProjectileData`] so both
/// the wasp-nest parent (`ObjectType::BonusWaspNest`) and each spawned
/// wasp (`ObjectType::Wasp`) can share the storage without a separate
/// entity variant.
///
/// `flying_wasp_count` is decremented when a wasp dies and is checked
/// each tick to decide whether to keep emitting the buzz sound.
/// `source_nest` is the back-pointer so the wasp can notify its nest
/// on death.
///
/// Wasp AI (chase/sting soldiers) lives in `engine::wasp_nest`; when a
/// wasp dies it decrements the nest's counter through `source_nest`.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct WaspData {
    /// On a wasp nest: remaining wasps in flight.  Incremented to
    /// `NUMBER_OF_WASPS` on burst, decremented when each wasp dies.
    /// On a wasp: always 0.
    pub flying_wasp_count: u32,
    /// On a wasp nest: true once the nest has burst and the 20 wasps
    /// have been spawned, to prevent re-bursting each tick.  On a
    /// wasp: always false.
    pub burst: bool,
    /// On a wasp: handle of the parent nest (for decrementing
    /// `flying_wasp_count` on death).  On a wasp nest: always `None`.
    pub source_nest: Option<EntityId>,
    /// On a wasp: handle of the currently targeted soldier, or `None`.
    /// Cleared when the distance exceeds `VICTIM_FORGET_DISTANCE` or
    /// when the wasp dies.
    pub victim: Option<EntityId>,
    /// On a wasp: `true` once the wasp has closed to `STING_DISTANCE`
    /// and committed to stinging its victim; movement stops and the
    /// sting-timeout counter runs down.
    pub stinging: bool,
    /// On a wasp: frame counter until next direction change (or, while
    /// stinging, until the sting fires).
    pub timeout: u32,
    /// On a wasp: current per-frame movement vector (3D velocity).
    /// Current per-frame 3D vector in raw world axes.
    pub movement: WorldVec3D,
}

// ═══════════════════════════════════════════════════════════════════
//  Concrete entity structs
// ═══════════════════════════════════════════════════════════════════

/// Player character entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActorPc {
    pub element: ElementData,
    pub actor: ActorData,
    pub human: HumanData,
    pub pc: PcData,
}

/// Soldier NPC entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActorSoldier {
    pub element: ElementData,
    pub actor: ActorData,
    pub human: HumanData,
    pub npc: NpcData,
    pub soldier: SoldierData,
}

/// Civilian NPC entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ActorCivilian {
    pub element: ElementData,
    pub actor: ActorData,
    pub human: HumanData,
    pub npc: NpcData,
    pub civilian: CivilianData,
}

/// Basic visual effect entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementFx {
    pub element: ElementData,
    pub fx: FxData,
}

/// Target / activator entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementTarget {
    pub element: ElementData,
    pub fx: FxData,
    pub target: TargetData,
}

/// Bonus / pickup object entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementBonus {
    pub element: ElementData,
    pub object: ObjectData,
}

/// Scroll (mission pickup) entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementScroll {
    pub element: ElementData,
    pub object: ObjectData,
    /// Per-difficulty presence flags (Easy/Medium/Hard).
    pub presence: [bool; 3],
    /// Tutorial flag — scrolls flagged as tutorial behave specially.
    pub tutorial: bool,
    /// Mission-stream script class name, bound at scroll init time.
    /// Empty when the scroll has no script.
    pub script_class: String,
    /// Counter (0..25) driving the per-scroll script `Hourglass(0)`
    /// dispatch.  Incremented every active tick; when it reaches 25
    /// the scroll's script `Hourglass` fires and the counter resets.
    pub script_hourglass_timeout: u32,
}

/// Projectile entity (arrows, stones, etc.).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementProjectile {
    pub element: ElementData,
    pub object: ObjectData,
    pub projectile: ProjectileData,
}

/// Net (trap net) entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ElementNet {
    pub element: ElementData,
    pub object: ObjectData,
    pub projectile: ProjectileData,
    pub net: NetData,
}

// ═══════════════════════════════════════════════════════════════════
//  Entity enum — dynamic dispatch over all entity types
// ═══════════════════════════════════════════════════════════════════

/// Any game entity.  Provides enum-based dispatch over all concrete types.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Entity {
    Pc(ActorPc),
    Soldier(ActorSoldier),
    Civilian(ActorCivilian),
    Fx(ElementFx),
    Target(ElementTarget),
    Bonus(ElementBonus),
    Scroll(ElementScroll),
    Projectile(ElementProjectile),
    Net(ElementNet),
}

/// Entity save projection retains sparse identity and enum layout while NPC
/// brains reconstruct their runtime-only continuation bookkeeping.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum PersistedEntity {
    Pc(PersistedActorPc),
    Soldier(PersistedActorSoldier),
    Civilian(PersistedActorCivilian),
    Fx(PersistedElementFx),
    Target(PersistedElementTarget),
    Bonus(PersistedElementBonus),
    Scroll(PersistedElementScroll),
    Projectile(PersistedElementProjectile),
    Net(PersistedElementNet),
}

impl PersistedEntity {
    pub(crate) fn capture(value: &Entity) -> Self {
        match value {
            Entity::Pc(value) => Self::Pc(PersistedActorPc::capture(value)),
            Entity::Soldier(value) => Self::Soldier(PersistedActorSoldier::capture(value)),
            Entity::Civilian(value) => Self::Civilian(PersistedActorCivilian::capture(value)),
            Entity::Fx(value) => Self::Fx(PersistedElementFx::capture(value)),
            Entity::Target(value) => Self::Target(PersistedElementTarget::capture(value)),
            Entity::Bonus(value) => Self::Bonus(PersistedElementBonus::capture(value)),
            Entity::Scroll(value) => Self::Scroll(PersistedElementScroll::capture(value)),
            Entity::Projectile(value) => {
                Self::Projectile(PersistedElementProjectile::capture(value))
            }
            Entity::Net(value) => Self::Net(PersistedElementNet::capture(value)),
        }
    }
    pub(crate) fn into_runtime(self) -> Entity {
        match self {
            Self::Pc(value) => Entity::Pc(value.into_runtime()),
            Self::Soldier(value) => Entity::Soldier(value.into_runtime()),
            Self::Civilian(value) => Entity::Civilian(value.into_runtime()),
            Self::Fx(value) => Entity::Fx(value.into_runtime()),
            Self::Target(value) => Entity::Target(value.into_runtime()),
            Self::Bonus(value) => Entity::Bonus(value.into_runtime()),
            Self::Scroll(value) => Entity::Scroll(value.into_runtime()),
            Self::Projectile(value) => Entity::Projectile(value.into_runtime()),
            Self::Net(value) => Entity::Net(value.into_runtime()),
        }
    }
}

/// Concrete original-game entity kind selected by the entity/object discriminants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginalHourglassClass {
    ActorPc,
    ActorSoldier,
    ActorCivilian,
    Fx,
    FxMasked,
    Target,
    Bonus,
    Ale,
    Cape,
    Scroll,
    Arrow,
    Apple,
    Stone,
    Purse,
    Coin,
    Net,
    WaspNest,
    Wasp,
}

// ═══════════════════════════════════════════════════════════════════
//  Trait hierarchy
// ═══════════════════════════════════════════════════════════════════

impl ActorSoldier {
    pub fn is_smelling_apple(&self) -> bool {
        self.soldier.apple_smell != 0
    }

    /// True when the soldier is in an observing attack substate
    /// (Observe / ObserveAndMove / LastReserve).  Non-Soldier entities
    /// should be treated as false.
    pub fn is_soldier_observing_swordfight(&self) -> bool {
        matches!(
            self.npc.ai_substate(),
            AiSubstate::AttackingObserve
                | AiSubstate::AttackingObserveAndMove
                | AiSubstate::AttackingLastReserve
        )
    }
}

impl ElementBonus {
    /// Original-game concrete element kind represented by this Rust `Entity::Bonus`.
    ///
    /// Rust deliberately shares one storage variant for several Original
    /// object kinds, so behavior dispatch must use the authored object
    /// type rather than assuming every value is a bonus element.
    pub fn original_concrete_class(&self) -> OriginalBonusConcreteClass {
        match self.object.object_type {
            ObjectType::Ale => OriginalBonusConcreteClass::Ale,
            ObjectType::Cape => OriginalBonusConcreteClass::Cape,
            ObjectType::BonusAmulet
            | ObjectType::BonusAle
            | ObjectType::BonusApple
            | ObjectType::BonusArrow
            | ObjectType::BonusBlazon
            | ObjectType::BonusLambLeg
            | ObjectType::BonusNet
            | ObjectType::BonusPlants
            | ObjectType::BonusPurse
            | ObjectType::BonusRansom
            | ObjectType::BonusStone
            | ObjectType::BonusWaspNest
            | ObjectType::BonusAmpulla
            | ObjectType::BonusCoronationSpoon
            | ObjectType::BonusRichardsCrown
            | ObjectType::BonusRoyalSeal
            | ObjectType::BonusRoyalSceptre
            | ObjectType::BonusDomesdayBook
            | ObjectType::BonusSwordOfTheState => OriginalBonusConcreteClass::Bonus,
            ObjectType::None
            | ObjectType::VirtualJumper
            | ObjectType::VirtualListen
            | ObjectType::Apple
            | ObjectType::Arrow
            | ObjectType::Stone
            | ObjectType::Purse
            | ObjectType::Coin
            | ObjectType::Net
            | ObjectType::Wasp
            | ObjectType::WaspNest
            | ObjectType::Scroll => OriginalBonusConcreteClass::Unsupported,
        }
    }

    pub fn is_relic(&self) -> bool {
        matches!(
            self.object.object_type,
            ObjectType::BonusAmpulla
                | ObjectType::BonusCoronationSpoon
                | ObjectType::BonusRichardsCrown
                | ObjectType::BonusRoyalSeal
                | ObjectType::BonusRoyalSceptre
                | ObjectType::BonusDomesdayBook
                | ObjectType::BonusSwordOfTheState
        )
    }

    /// Whether this bonus item can be picked up by a PC.
    pub fn is_takable(&self) -> bool {
        !self.object.taken && self.element.active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalBonusConcreteClass {
    Bonus,
    Ale,
    Cape,
    /// TODO(original-parity): add a mapping only when original-game initialization
    /// proves that the object type can inhabit Rust's `Entity::Bonus`.
    Unsupported,
}

impl ElementProjectile {
    /// Run the movement portion of the projectile update.
    /// Every dispatched update snapshots `old_position` first; keeping that edge
    /// here prevents purse/coin callers from advancing with a stale creation
    /// default while arrow callers use a different implementation.
    pub fn advance_projectile_hourglass(&mut self) -> bool {
        self.element.sprite.position_iface.new_move();
        self.advance_trajectory_one_frame()
    }

    /// Advance the projectile by one trajectory frame: pop the next
    /// waypoint when the current segment timer expires, then apply the
    /// per-frame velocity increment to the position / map / direction.
    ///
    /// Returns `true` when the trajectory was exhausted on this call
    /// (the caller is responsible for the resulting impact handling —
    /// e.g. arrow despawn, coin landing, purse burst).  When the
    /// trajectory is exhausted the `flying` flag is cleared.
    ///
    /// The bow-shot tick already inlines this for arrows; the helper
    /// exists so the purse / coin path can call into the same logic
    /// (coin obstacle-impact handling runs an update once before
    /// registration).
    pub fn advance_trajectory_one_frame(&mut self) -> bool {
        advance_trajectory_one_frame(&mut self.element, &mut self.projectile)
    }
}

/// Free-function form of [`ElementProjectile::advance_trajectory_one_frame`]
/// so [`ElementNet`] (which has the same `element` + `projectile` fields plus
/// extra net state) can share the same step-one-trajectory-waypoint logic.
pub fn advance_trajectory_one_frame(
    element: &mut ElementData,
    projectile: &mut ProjectileData,
) -> bool {
    if projectile.trajectory_frame_count == 0 {
        if projectile.trajectory.is_empty() {
            projectile.flying = false;
            projectile.trajectory_frame_count = u16::MAX;
            projectile.velocity_increment = WorldVec3D::ZERO;
            return true;
        }
        if !projectile.trajectory_runtime.is_empty() {
            assert_eq!(
                projectile.trajectory_runtime.len(),
                projectile.trajectory.len(),
                "projectile trajectory/runtime arrays lost their serialized lockstep"
            );
            let runtime = projectile.trajectory_runtime.remove(0);
            element.set_material(crate::element::GameMaterial::from_u32(runtime.material));
        }
        let point = projectile.trajectory.remove(0);
        let time = point.time.max(1);
        projectile.trajectory_frame_count = time - 1;
        let current = element.position();
        let factor = 1.0 / time as f32;
        projectile.velocity_increment = WorldVec3D {
            x: (point.position.x - current.x) * factor,
            y: (point.position.y - current.y) * factor,
            z: (point.position.z - current.z) * factor,
        };
        projectile.end = point.position;
    } else {
        projectile.trajectory_frame_count -= 1;
    }

    let mut p = element.position();
    p.x += projectile.velocity_increment.x;
    p.y += projectile.velocity_increment.y;
    p.z += projectile.velocity_increment.z;
    element.set_position(p);
    element.set_position_map_preserving_3d(MapPoint::from_world_xyz(p.x, p.y, p.z));
    element.finish_projectile_position_update(projectile.velocity_increment);
    let vx = projectile.velocity_increment.x;
    let vy = projectile.velocity_increment.y;
    if projectile.frame_count == 0 && (vx != 0.0 || vy != 0.0) {
        projectile.flight_direction =
            crate::position_interface::vector_to_sector_0_to_15_iso(vx, vy) as u16;
    }

    projectile.frame_count = projectile.frame_count.saturating_add(1);
    false
}

impl ElementNet {
    /// Step the net's ballistic trajectory one frame.  Shares the same
    /// logic as [`ElementProjectile::advance_trajectory_one_frame`].
    pub fn advance_trajectory_one_frame(&mut self) -> bool {
        advance_trajectory_one_frame(&mut self.element, &mut self.projectile)
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "element/tests.rs"]
mod tests;
