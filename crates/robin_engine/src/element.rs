//! Entity hierarchy — the base class system for all game entities.
//!
//! The animal actor branch is *not* ported — no shipped Robin Hood level
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
//! - **Traits**: Virtual method interfaces via trait inheritance
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

    /// Original `RHElement`'s queued map-space teleport. Actor Hourglass
    /// consumes this before selecting/executing the current order.
    pub position_map_delayed: bool,
    pub delayed_map_position: MapPoint,

    /// Original `RHElement`'s queued world-space teleport. When both queues
    /// are armed, Actor Hourglass consumes the map-space queue first and
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
    /// corpse-transition guard stays centralized. TODO: the field is
    /// still public for broad read compatibility; narrow that once the
    /// remaining read churn is practical.
    pub posture: Posture,

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

    /// Queue a map-space position change for the next Actor Hourglass.
    pub fn set_position_map_delayed(&mut self, point: MapPoint) {
        self.delayed_map_position = point;
        self.position_map_delayed = true;
    }

    /// Queue a world-space position change for the next Actor Hourglass.
    pub fn set_position_delayed(&mut self, point: WorldPoint3D) {
        self.delayed_position = point;
        self.position_delayed = true;
    }

    /// Apply the next queued Original position update, preserving the
    /// map-before-world priority and the inactive queue's stored point.
    ///
    /// Returns the pre/post map segment and layer for the Actor
    /// `CheckForLineCrossing` continuation.
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
    pub fn set_layer(&mut self, l: u16) {
        let layer = crate::position_interface::Layer::new(l)
            .expect("layer must be < 0xFFFF; 0xFFFF is the 'no layer' sentinel");
        self.sprite.position_iface.set_layer(layer);
    }
    #[inline]
    pub fn clear_layer(&mut self) {
        self.sprite
            .position_iface
            .set_layer(crate::position_interface::Layer::from_saved_raw(u16::MAX));
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

    /// Door-transit half of "is inside a building": true while the
    /// actor is in the middle of a pass-door animation, before its
    /// sector pointer has been swapped to the inside-building sector.
    #[inline]
    pub fn is_in_door_transit(&self) -> bool {
        !self.sprite.position_iface.get_door().is_null()
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

    /// Distance from the actor's map position to the boundary of the
    /// material sector it is standing in, under the 1-norm with Y
    /// pre-stretched by `INVERSE_ASPECT_RATIO`.  Returns 0 if the
    /// actor is not inside any material sector.  Caller: debug
    /// material-sector overlay (unported).
    ///
    /// The containing sector is located at query time rather than
    /// cached on the actor — the only caller is a debug HUD that runs
    /// once per frame, so caching would be a micro-optimisation.  This
    /// method has no active users yet; it exists so reviving the
    /// overlay is a one-line change.
    /// Change the posture, respecting the corpse-transition guard: a
    /// `Dead` / `DeadBack` corpse can only transition to `Carried`
    /// (pickup); any other posture write on a dead sprite is silently
    /// dropped. This is the single runtime mutation point for
    /// `posture`; direct field writes should go through here.
    ///
    /// The "fire intersection update on every lying↔non-lying
    /// transition" hook is implemented as a deferred per-tick drain
    /// rather than a synchronous hook on this setter: the hook needs
    /// engine access to iterate actors. See
    /// [`EngineInner::process_corpse_intersection_updates`].
    pub fn set_posture(&mut self, p: Posture) {
        if self.posture.allows_transition_to(p) {
            self.posture = p;
        }
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
    /// ReadyForTakeOff resolved a complete world-space endpoint, including
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
/// `ExecuteRiderCharge`; origin, direction, layer, and animation frame are
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
        /// `GetDistanceForAnimation`).  Stairs / building door passes
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
    /// `StartHulk(OCN_DEFAULT, 2, true, tolerance)`.
    Select {
        /// Speed factor for the hulk fade —
        /// `(pMid - pOut/pIn).Norm() * 0.03`.
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

/// Exact Rust mirror of Original's installed `RHElementActor::mpOrder`.
///
/// Sequence-manager selection is deliberately not sufficient: an element can
/// be selected before `Instruct` installs its first order, while `DoNextOrder`
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

    /// Actor::Hourglass-selected order identity, corresponding to Original
    /// `mulLastOrderID`. This is deliberately independent of the sprite's
    /// processed order: FrozenAll still consumes actor initialization once.
    pub last_execute_order_id: Option<std::num::NonZeroU32>,
    /// Original's live `mpOrder` pointer identity and action.
    pub installed_order: Option<InstalledActorOrder>,
    /// Original `mbNewOrder` for the currently-entered Execute call. Set at
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
    /// Original `RHElementActor::mfSeekDistance`: the unadapted distance
    /// requested by the transient Seek command. Moving-target refreshes
    /// derive their concrete tolerance from this stable base each time.
    pub seek_distance: f32,
    /// Countdown before the actor may re-issue a seek against a moving
    /// target.  Armed to `TIME_SEEK_REFRESH` (25) at seek launch and
    /// after each RefreshSeek; decremented by entity-target `PerformSeek`
    /// owner execution when that call does not return through a successful
    /// post-seek arrival.
    pub seek_refresh_wait: u32,
    // Note: concrete seek tolerance/flags/sector/layer live on the active
    // `Movement` element. `seek_distance` is deliberately actor-owned
    // because RefreshSeek repeatedly derives concrete moving-target
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
    /// `ExecuteRiderCharge` — moving along a path while checking a
    /// polygon hit zone each frame.
    pub active_rider_charge: Option<ActiveRiderCharge>,

    /// Actor-level identity of the last `RiderCharging` order whose
    /// `ExecuteRiderCharge` pass completed. This is intentionally distinct
    /// from `Sprite::last_processed_order_id`: FrozenAll still executes the
    /// charge polygon but must leave sprite motion initialization pending.
    pub last_executed_rider_charge_order_id: Option<std::num::NonZeroU32>,

    /// 3D bounding-box obstacle representing the shield held in front of
    /// this actor. Refreshed only at the Original's explicit `UpdateShield`
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
    /// Persistent `RHElementActorHuman::mboxHearMyNoiseBox` state.
    ///
    /// Original `RefreshProducedNoise` updates the noise origin before its
    /// animation switch, but its inactive/building and quiet-animation arms
    /// return before rebuilding this box.  Hearing therefore deliberately
    /// tests a stale box in those cases instead of deriving one from the
    /// current volume and origin.
    pub hear_noise_box: MapBBox,
    /// Complete produced-noise record from this PC's most recently visited
    /// Human Hourglass slot. NPCs earlier in creation order observe this
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

/// Exact mutable `RHRepulsivePoint` state owned by a Human.
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

/// Exact serialized `RHSightObstacle` owned by a Human.
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

/// One entry in the Original's ordered `RHSwordfightOpponent` list.
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
    /// Original `RHSwordfightOpponent::pJumpLine`.
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
/// The Original stores a single `SBList<RHSwordfightOpponent>`. Older Rust
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

    /// Match the Original's `AddOpponent`: insert a new principal entry, or
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
    /// `CHECKENUM(mCurrentlyProducedNoise)` in the Original accidentally
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
/// The original `RHElementActorPC` always has an `RHPCStatus` pointer, and
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
)]
pub struct PcPortraitQuickIconState {
    pub titbit_id: u32,
    pub running: bool,
}

impl Default for PcPortraitQuickIconState {
    fn default() -> Self {
        Self {
            titbit_id: u32::MAX,
            running: false,
        }
    }
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
    /// Exact index of Original's `mpDescription` pointer in
    /// `RHCampaign::mapPCDescription`.
    ///
    /// This is independent of `mubListIndex`: multiple campaign
    /// descriptions may share one character profile, while the list byte is
    /// actor/UI state serialized separately by `RHElementActorPC`.
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
    pub titbits: Vec<u32>,
    pub portrait: PcPortraitState,

    // Detection
    pub head_seen: bool,
    pub belt_seen: bool,
    pub feet_seen: bool,

    // Teleport
    pub position_before_teleport: MapPoint,
    /// Frames remaining in the cheat-teleport hulk-rebuild fade.
    /// Decremented each frame by the per-PC render path (not yet
    /// ported); read here by `SetTeleportStuff` to suppress the
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
    /// Raw Original `mCarriedPosture` storage.
    ///
    /// The Original constructor leaves it indeterminate while `carried` is
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
    /// Evaluated by `InitializeAction()` to set the PC's starting
    /// state.
    pub initial_action: u32,

    /// Forbidden hero expression list (expression_id, forbid_timer).
    /// Each entry counts down each frame and is removed at 0, preventing
    /// the same expression from repeating too quickly.
    pub forbidden_expressions: Vec<(u16, u16)>,

    /// Last `combat_anim` id observed by the speech-trigger tick — used
    /// to detect the START of a new animation and the DONE transition
    /// (anim cleared) for `DoActionAndThenPlayRemark`.
    pub prev_combat_anim_id: u32,
    pub prev_combat_anim_ot: Option<crate::order::OrderType>,
}

impl Default for PcData {
    fn default() -> Self {
        Self {
            life_points: 100,
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
            titbits: vec![u32::MAX; 3],
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
    /// Apply Original's `RHElementActorPC::SetPlayable` state change.
    ///
    /// In retail missions, making a PRIS rescue PC playable is also the
    /// boundary where that scripted prisoner becomes an ordinary party hero.
    /// The original class hierarchy made those facts implicit; keep the
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
                "live carried_posture contains invalid Original enum word {}",
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
    /// Original `RHElementActorNPC` assigns this from
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
    /// engine fires `ForceReturnToDuty` so NPCs that hang on outdoor
    /// ladders can self-recover.
    pub stuck_on_ladder_emergency_counter: u16,

    /// Exact dialog-scroll entity attached by Original's
    /// `RHElementActorNPC::mpAttachedScroll`.
    ///
    /// Keeping the identity (rather than only a boolean) matters when a
    /// loaded NPC is clicked: the interaction must open the same scroll
    /// object which was serialized in the save.
    pub attached_scroll: Option<EntityId>,

    /// Original's serialized `muwBodyVisitors` continuation counter.
    pub body_visitors: u16,

    /// Original's serialized `mbFriedPikachu` script latch. Original stores
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
    /// base value; the per-frame mutation logic in `RefreshView` is
    /// not yet ported.
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
    /// **Mutation is not yet fully ported.**  The value stays at the
    /// initial `NORMAL_HALF_APERTURE` until the RefreshView logic
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
    /// block is disabled in the shipped Original source, but these members
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
    /// visibility tests until `RefreshView` computes the next pair.
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
    /// Original `mViewParameters.starePoint`: world-ground `(x, y)`, not
    /// projected map coordinates. `Focus(RHposition)` and `EYES_FOLLOW`
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
            // view radius during level load via `InitViewRadius()`,
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
            // RHElementActorNPC initializes uwRadiusStep to 10.  The
            // DieOrGetUnconscious view-cone collapse consumes this value on
            // its first RefreshView and then accelerates it by 5 each frame.
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
    /// Original `RHElementActorNPC::DeleteDetectable`: remove only the first
    /// matching entry and report whether one was found. Release recordings can
    /// contain duplicates because several Original `AddDetectable` callers only
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
            life_points: 100,
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
    /// pass of `SortForDisplay` (characters can walk behind them).
    pub display_polyline: Vec<MapPoint>,
    /// If this FX entity is a patch's animation element, the patch
    /// index (into the canonical interactable patch table). Used by the animation tick
    /// to apply reversed playback and detect transition completion.
    pub patch_index: Option<crate::patch::PatchIndex>,
    /// Index of the owning mission mobile element for its masked child
    /// sprite. `None` for ordinary proto/patch FX.
    #[serde(default)]
    pub mobile_index: Option<u16>,
    /// C++ `RHElementFXMasked::mfAnimationSpeed`. Mobile carts update this
    /// to `1 / movement_speed`; ordinary FX retain `1.0`.
    #[serde(default = "default_fx_animation_speed")]
    pub animation_speed: f32,
    /// Rendering properties (`Blocky` vs `NeedShadow`) selected from
    /// the blit-type byte at level load.  `NeedShadow` selects the
    /// `BlitAlphaKeying` path (sprite gets the global shadow tint
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

/// Bit-exact dormant storage for an Original v48 object's embedded point.
///
/// Original's default `RHRepulsivePoint` constructor leaves its four force
/// scalars uninitialized. `RHElementObject` nevertheless serializes the whole
/// point, including those dormant bytes. Keep their exact IEEE-754 storage
/// without exposing non-finite values to runtime geometry or JSON snapshots.
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
pub struct LegacyV48ObjectRepulsivePointState {
    pub position_bits: [u32; 2],
    pub concave: bool,
    pub limit_left_bits: [u32; 2],
    pub limit_right_bits: [u32; 2],
    pub action_radius_bits: u32,
    pub force_a_bits: u32,
    pub force_b_bits: u32,
    pub radius_bits: u32,
    pub id: u32,
    pub affects_pcs: bool,
    pub affects_soldiers: bool,
    pub affects_civilians: bool,
    pub affects_animals: bool,
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
    /// Exact serialized storage for the embedded Original v48 repulsive point.
    ///
    /// This is compatibility state, not collision input. Original overwrites
    /// position and force before exposing the point, or omits it for objects
    /// with no radius. Rust collision code must likewise use live object
    /// geometry rather than these constructor residues.
    #[serde(default)]
    #[state_hash(skip)]
    pub legacy_v48_repulsive_point: Option<LegacyV48ObjectRepulsivePointState>,
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
            legacy_v48_repulsive_point: None,
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
    /// Original's water/hole landing latch (`mbDive`).
    #[serde(default)]
    pub dive: bool,
    /// Bloodseeker-oil straight-flight latch (`mbMagicBullet`).
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
    /// exact collision waypoint materialized before the first Hourglass.
    /// Cleared before publication by the purse/coin constructor boundary.
    #[serde(default)]
    pub terminal_material_pending: bool,
    /// Exact raw collision waypoint retained by ComputeTrajectory until the
    /// pre-publication virtual Hourglass can determine its material.
    #[serde(default)]
    pub terminal_material_impact_index: Option<u16>,
    #[serde(default)]
    pub trajectory_origin_sector: Option<u16>,
    /// Exact arena half of Original's `mposStartOfTrajectory.pSector`.
    /// The public number above remains for backward-compatible serialized
    /// state, but AI projectile-hit callbacks must copy the complete sector
    /// pointer identity into their stimulus position.
    #[serde(default)]
    pub trajectory_origin_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    #[serde(default)]
    pub trajectory_origin_layer: u16,
    /// Per-frame position delta for the current trajectory segment.
    /// Recomputed each time a new waypoint is popped.
    pub velocity_increment: WorldVec3D,
    /// Original `muwFlightDirection`, sampled from the initial trajectory
    /// velocity. This is gameplay state used to orient hit actors; it is
    /// deliberately separate from the projectile element's sprite facing.
    #[serde(default)]
    pub flight_direction: u16,
    /// Optional explicit start point for the next collision segment.
    /// Used when a projectile is advanced once before entering the
    /// engine list, matching C++ `pProjectile->Hourglass()` at spawn.
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
    /// `RHElementArrow::Refresh`. Original serializes this cache and reuses it
    /// when the trajectory has become empty.
    #[serde(default)]
    pub last_orientation_sector: u16,
    /// Last non-falling vertical orientation, in degrees in `[-60, 60]`.
    #[serde(default)]
    pub last_orientation_azimuth: i16,
    /// Sector (0..15) used by a falling arrow's visual rotation. Cycled by
    /// the deferred `RHElementArrow::Refresh` boundary, not by Hourglass.
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

impl Default for ProjectileData {
    fn default() -> Self {
        Self {
            start: WorldPoint3D::default(),
            end: WorldPoint3D::default(),
            start_of_trajectory_x: 0.0,
            start_of_trajectory_y: 0.0,
            shooter: None,
            frame_count: 0,
            flying: false,
            dive: false,
            magic_bullet: false,
            disappear: false,
            trajectory: Vec::new(),
            trajectory_runtime: Vec::new(),
            terminal_material_pending: false,
            terminal_material_impact_index: None,
            trajectory_origin_sector: None,
            trajectory_origin_sector_index: None,
            trajectory_origin_layer: 0,
            velocity_increment: WorldVec3D::default(),
            flight_direction: 0,
            launch_segment_start: None,
            trajectory_frame_count: 0,
            damage: 0,
            falling: false,
            last_orientation_sector: 0,
            last_orientation_azimuth: 0,
            falling_direction: 0,
            arrow_bow_profile: None,
            arrow_flat_shot: false,
            arrow_play_impact: false,
            purse: PurseData::default(),
            wasp: WaspData::default(),
        }
    }
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
    /// On a purse: list of child coins spawned by `HitObstacle`.  On
    /// a coin: empty.
    pub child_coins: Vec<EntityId>,
    /// On a purse: number of coins still owed (decremented each burst).
    /// Initialised to `inventory::COINS_PER_PURSE` at spawn.  On a
    /// coin: always 0.
    pub number_of_coins: u16,
    /// On a purse: true once `HitObstacle` has fired and child coins
    /// are in flight.  Used by `Hourglass` to know when to switch to
    /// the "drain children, then despawn" idle phase.
    pub burst: bool,
    /// On a coin: layer the coin should snap to on landing.  Stored
    /// at spawn so `HitObstacle` can re-key the coin onto its goal
    /// layer.  On a purse: always 0.
    pub layer_goal: u16,
    /// On a coin: sector the coin should snap to on landing (None
    /// when the scatter target wasn't resolved against a known
    /// sector).
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

/// Helper macro — dispatch `$self` to the `element` field of every variant.
macro_rules! dispatch_element {
    ($self:expr_2021, $field:ident) => {
        match $self {
            Entity::Pc(e) => &e.element.$field,
            Entity::Soldier(e) => &e.element.$field,
            Entity::Civilian(e) => &e.element.$field,
            Entity::Fx(e) => &e.element.$field,
            Entity::Target(e) => &e.element.$field,
            Entity::Bonus(e) => &e.element.$field,
            Entity::Scroll(e) => &e.element.$field,
            Entity::Projectile(e) => &e.element.$field,
            Entity::Net(e) => &e.element.$field,
        }
    };
}

macro_rules! entity_variant_accessors {
    ($as_ref:ident, $as_mut:ident, $variant:ident, $entity:ty) => {
        pub fn $as_ref(&self) -> Option<&$entity> {
            match self {
                Self::$variant(entity) => Some(entity),
                _ => None,
            }
        }

        pub fn $as_mut(&mut self) -> Option<&mut $entity> {
            match self {
                Self::$variant(entity) => Some(entity),
                _ => None,
            }
        }
    };
}

impl Entity {
    /// Resolve the Original concrete class whose virtual `Hourglass` chain
    /// owns this Rust entity.
    ///
    /// `Entity`, `ElementKind`, and (for objects) `ObjectType` jointly replace
    /// the Original class-id/vtable.  Keep the match exhaustive so adding a
    /// Rust kind or object type cannot silently inherit an unrelated update.
    pub(crate) fn original_hourglass_class(&self) -> OriginalHourglassClass {
        let (expected_kind, class) = match self {
            Self::Pc(_) => (ElementKind::ActorPc, OriginalHourglassClass::ActorPc),
            Self::Soldier(_) => (
                ElementKind::ActorSoldier,
                OriginalHourglassClass::ActorSoldier,
            ),
            Self::Civilian(_) => (
                ElementKind::ActorCivilian,
                OriginalHourglassClass::ActorCivilian,
            ),
            Self::Fx(fx) => (
                ElementKind::Fx,
                if fx.fx.mobile_index.is_some() {
                    OriginalHourglassClass::FxMasked
                } else {
                    OriginalHourglassClass::Fx
                },
            ),
            Self::Target(_) => (ElementKind::Target, OriginalHourglassClass::Target),
            Self::Bonus(bonus) => match bonus.object.object_type {
                ObjectType::Ale => (ElementKind::ObjectOther, OriginalHourglassClass::Ale),
                // RHElementCape.cpp constructs RHElementObject with
                // OBJECT_BONUS even though Cape has its own virtual class.
                ObjectType::Cape => (ElementKind::ObjectBonus, OriginalHourglassClass::Cape),
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
                | ObjectType::BonusSwordOfTheState => {
                    (ElementKind::ObjectBonus, OriginalHourglassClass::Bonus)
                }
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
                | ObjectType::Scroll => panic!(
                    "Entity::Bonus has no Original concrete-class mapping for ObjectType::{:?}",
                    bonus.object.object_type
                ),
            },
            Self::Scroll(scroll) => match scroll.object.object_type {
                ObjectType::Scroll => (ElementKind::ObjectScroll, OriginalHourglassClass::Scroll),
                object_type => panic!(
                    "Entity::Scroll has invalid ObjectType::{object_type:?}; expected Scroll"
                ),
            },
            Self::Projectile(projectile) => {
                let class = match projectile.object.object_type {
                    ObjectType::Arrow => OriginalHourglassClass::Arrow,
                    ObjectType::Apple => OriginalHourglassClass::Apple,
                    ObjectType::Stone => OriginalHourglassClass::Stone,
                    ObjectType::Purse => OriginalHourglassClass::Purse,
                    ObjectType::Coin => OriginalHourglassClass::Coin,
                    ObjectType::WaspNest | ObjectType::BonusWaspNest => {
                        OriginalHourglassClass::WaspNest
                    }
                    ObjectType::Wasp => OriginalHourglassClass::Wasp,
                    ObjectType::None
                    | ObjectType::VirtualJumper
                    | ObjectType::VirtualListen
                    | ObjectType::Ale
                    | ObjectType::Net
                    | ObjectType::Scroll
                    | ObjectType::Cape
                    | ObjectType::BonusAmulet
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
                    | ObjectType::BonusAmpulla
                    | ObjectType::BonusCoronationSpoon
                    | ObjectType::BonusRichardsCrown
                    | ObjectType::BonusRoyalSeal
                    | ObjectType::BonusRoyalSceptre
                    | ObjectType::BonusDomesdayBook
                    | ObjectType::BonusSwordOfTheState => panic!(
                        "Entity::Projectile has no Original concrete-class mapping for ObjectType::{:?}",
                        projectile.object.object_type
                    ),
                };
                (ElementKind::ObjectProjectile, class)
            }
            Self::Net(net) => match net.object.object_type {
                ObjectType::Net | ObjectType::BonusNet => {
                    (ElementKind::ObjectNet, OriginalHourglassClass::Net)
                }
                object_type => panic!(
                    "Entity::Net has invalid ObjectType::{object_type:?}; expected Net or BonusNet"
                ),
            },
        };
        assert_eq!(
            self.kind(),
            expected_kind,
            "Rust entity variant/ElementKind invariant failed for Original {class:?}"
        );
        class
    }

    pub fn entity_id_kind(&self) -> EntityIdKind {
        match self {
            Self::Pc(_) => EntityIdKind::Pc,
            Self::Soldier(_) => EntityIdKind::Soldier,
            Self::Civilian(_) => EntityIdKind::Civilian,
            Self::Fx(_) => EntityIdKind::Fx,
            Self::Target(_) => EntityIdKind::Target,
            Self::Bonus(_) => EntityIdKind::Bonus,
            Self::Scroll(_) => EntityIdKind::Scroll,
            Self::Projectile(_) => EntityIdKind::Projectile,
            Self::Net(_) => EntityIdKind::Net,
        }
    }

    entity_variant_accessors!(as_pc, as_pc_mut, Pc, ActorPc);
    entity_variant_accessors!(as_soldier, as_soldier_mut, Soldier, ActorSoldier);
    entity_variant_accessors!(as_civilian, as_civilian_mut, Civilian, ActorCivilian);
    entity_variant_accessors!(as_fx, as_fx_mut, Fx, ElementFx);
    entity_variant_accessors!(as_target, as_target_mut, Target, ElementTarget);
    entity_variant_accessors!(as_bonus, as_bonus_mut, Bonus, ElementBonus);
    entity_variant_accessors!(as_scroll, as_scroll_mut, Scroll, ElementScroll);
    entity_variant_accessors!(
        as_projectile,
        as_projectile_mut,
        Projectile,
        ElementProjectile
    );
    entity_variant_accessors!(as_net, as_net_mut, Net, ElementNet);

    // — Element data access —

    pub fn element_data(&self) -> &ElementData {
        match self {
            Self::Pc(e) => &e.element,
            Self::Soldier(e) => &e.element,
            Self::Civilian(e) => &e.element,
            Self::Fx(e) => &e.element,
            Self::Target(e) => &e.element,
            Self::Bonus(e) => &e.element,
            Self::Scroll(e) => &e.element,
            Self::Projectile(e) => &e.element,
            Self::Net(e) => &e.element,
        }
    }

    pub fn element_data_mut(&mut self) -> &mut ElementData {
        match self {
            Self::Pc(e) => &mut e.element,
            Self::Soldier(e) => &mut e.element,
            Self::Civilian(e) => &mut e.element,
            Self::Fx(e) => &mut e.element,
            Self::Target(e) => &mut e.element,
            Self::Bonus(e) => &mut e.element,
            Self::Scroll(e) => &mut e.element,
            Self::Projectile(e) => &mut e.element,
            Self::Net(e) => &mut e.element,
        }
    }

    pub fn kind(&self) -> ElementKind {
        *dispatch_element!(self, kind)
    }

    pub fn is_active(&self) -> bool {
        self.element_data().active
    }

    pub fn posture(&self) -> Posture {
        self.element_data().posture
    }

    /// Set posture through the corpse-transition guard.  Delegates to
    /// [`ElementData::set_posture`].
    pub fn set_posture(&mut self, p: Posture) {
        self.element_data_mut().set_posture(p);
    }

    /// Reveal a blipped shadow.  Pulls `direction` from the actor's
    /// `PositionInterface` and delegates to
    /// [`ElementData::reveal_blip`].
    pub fn reveal_blip(&mut self) {
        let direction = (self.position_iface().get_direction().as_u8()) as u16;
        self.element_data_mut().reveal_blip(direction);
    }

    // — Sub-data accessors (return None if the entity doesn't have that level) —

    pub fn actor_data(&self) -> Option<&ActorData> {
        match self {
            Self::Pc(e) => Some(&e.actor),
            Self::Soldier(e) => Some(&e.actor),
            Self::Civilian(e) => Some(&e.actor),
            _ => None,
        }
    }

    pub fn actor_data_mut(&mut self) -> Option<&mut ActorData> {
        match self {
            Self::Pc(e) => Some(&mut e.actor),
            Self::Soldier(e) => Some(&mut e.actor),
            Self::Civilian(e) => Some(&mut e.actor),
            _ => None,
        }
    }

    pub fn human_data(&self) -> Option<&HumanData> {
        match self {
            Self::Pc(e) => Some(&e.human),
            Self::Soldier(e) => Some(&e.human),
            Self::Civilian(e) => Some(&e.human),
            _ => None,
        }
    }

    /// Stable content/body archetype. This deliberately says nothing about
    /// allegiance, player commandability, or who owns moment-to-moment
    /// decisions.
    pub fn human_archetype(&self) -> Option<HumanArchetype> {
        match self {
            Self::Pc(_) => Some(HumanArchetype::Hero),
            Self::Soldier(_) => Some(HumanArchetype::Soldier),
            Self::Civilian(_) => Some(HumanArchetype::Civilian),
            _ => None,
        }
    }

    pub fn decision_policy(&self) -> Option<DecisionPolicy> {
        let from_brain = |brain: &AiBrain| match brain {
            AiBrain::Enemy(_) => Some(DecisionPolicy::EnemyAi),
            AiBrain::Friendly(_) => Some(DecisionPolicy::FriendlyAi),
            AiBrain::None => None,
        };
        match self {
            Self::Pc(hero) => hero
                .pc
                .ai
                .as_deref()
                .and_then(|ai| from_brain(&ai.ai_brain))
                .or(Some(
                    if hero.pc.command_interface == CommandInterface::HeroActions {
                        DecisionPolicy::PlayerDirected
                    } else {
                        DecisionPolicy::Scripted
                    },
                )),
            Self::Soldier(soldier) => {
                from_brain(&soldier.npc.ai.ai_brain).or(Some(DecisionPolicy::Scripted))
            }
            Self::Civilian(civilian) => {
                from_brain(&civilian.npc.ai.ai_brain).or(Some(DecisionPolicy::Scripted))
            }
            _ => None,
        }
    }

    pub fn command_interface(&self) -> Option<CommandInterface> {
        match self {
            Self::Pc(hero) => Some(hero.pc.command_interface),
            Self::Soldier(soldier) => Some(soldier.soldier.command_interface),
            Self::Civilian(_) => Some(CommandInterface::None),
            _ => None,
        }
    }

    pub fn mission_role(&self) -> Option<MissionRole> {
        match self {
            Self::Pc(hero) => Some(hero.pc.mission_role),
            Self::Soldier(soldier) => Some(soldier.soldier.mission_role),
            Self::Civilian(_) => Some(MissionRole::Civilian),
            _ => None,
        }
    }

    pub fn combat_stance(&self) -> Option<CombatStance> {
        match self {
            Self::Pc(hero) => Some(hero.pc.combat_stance),
            Self::Soldier(soldier) => Some(soldier.soldier.combat_stance),
            Self::Civilian(_) => Some(CombatStance::Defensive),
            _ => None,
        }
    }

    pub fn human_control_profile(&self) -> Option<HumanControlProfile> {
        Some(HumanControlProfile {
            archetype: self.human_archetype()?,
            decision_policy: self.decision_policy()?,
            command_interface: self.command_interface()?,
            mission_role: self.mission_role()?,
            combat_stance: self.combat_stance()?,
        })
    }

    pub fn is_ai_controlled_human(&self) -> bool {
        matches!(
            self.decision_policy(),
            Some(DecisionPolicy::EnemyAi | DecisionPolicy::FriendlyAi)
        )
    }

    pub fn accepts_hero_commands(&self) -> bool {
        self.command_interface() == Some(CommandInterface::HeroActions)
    }

    pub fn accepts_tactical_orders(&self) -> bool {
        self.command_interface() == Some(CommandInterface::TacticalOrders)
    }

    pub fn npc_data(&self) -> Option<&NpcData> {
        match self {
            Self::Soldier(e) => Some(&e.npc),
            Self::Civilian(e) => Some(&e.npc),
            _ => None,
        }
    }

    /// Perception and decision state for every actor which owns an AI brain.
    /// This includes ordinary NPCs and opt-in AI-controlled heroes.
    pub fn ai_actor_data(&self) -> Option<&AiActorData> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref(),
            Self::Soldier(e) => Some(&e.npc.ai),
            Self::Civilian(e) => Some(&e.npc.ai),
            _ => None,
        }
    }

    pub fn human_data_mut(&mut self) -> Option<&mut HumanData> {
        match self {
            Self::Pc(e) => Some(&mut e.human),
            Self::Soldier(e) => Some(&mut e.human),
            Self::Civilian(e) => Some(&mut e.human),
            _ => None,
        }
    }

    /// Get mutable references to both HumanData and life points simultaneously.
    ///
    /// These live on disjoint fields (human vs pc/npc), so both can be
    /// borrowed mutably at the same time — the single match arm proves
    /// non-aliasing to the borrow checker.
    pub fn human_and_life_points_mut(&mut self) -> Option<(&mut HumanData, &mut i16)> {
        match self {
            Self::Pc(e) => Some((&mut e.human, &mut e.pc.life_points)),
            Self::Soldier(e) => Some((&mut e.human, &mut e.npc.life_points)),
            Self::Civilian(e) => Some((&mut e.human, &mut e.npc.life_points)),
            _ => None,
        }
    }

    /// Get mutable references to both HumanData and the entity's Posture simultaneously.
    ///
    /// These live on disjoint fields (human vs element.posture), so both can be
    /// borrowed mutably at the same time.
    ///
    /// Test-only: production posture mutations must go through
    /// [`Entity::set_posture`] or the focused entity helpers below.
    #[cfg(test)]
    pub fn human_and_posture_mut(&mut self) -> Option<(&mut HumanData, &mut Posture)> {
        match self {
            Self::Pc(e) => Some((&mut e.human, &mut e.element.posture)),
            Self::Soldier(e) => Some((&mut e.human, &mut e.element.posture)),
            Self::Civilian(e) => Some((&mut e.human, &mut e.element.posture)),
            _ => None,
        }
    }

    pub fn set_posture_stuck_under_net_for_human(&mut self) -> bool {
        match self {
            Self::Pc(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            Self::Soldier(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            Self::Civilian(e) => {
                e.element.set_posture(Posture::StuckUnderNet);
                true
            }
            _ => false,
        }
    }

    pub fn remove_net_from_human(&mut self) -> bool {
        fn apply(element: &mut ElementData, human: &mut HumanData) -> bool {
            let prev_counter = human.stuck_under_nets_counter;
            if human.stuck_under_nets_counter > 0 {
                human.stuck_under_nets_counter -= 1;
            }
            if human.stuck_under_nets_counter == 0 && element.posture == Posture::StuckUnderNet {
                element.set_posture(Posture::Lying);
            }
            prev_counter > 0 && human.stuck_under_nets_counter == 0
        }

        match self {
            Self::Pc(e) => apply(&mut e.element, &mut e.human),
            Self::Soldier(e) => apply(&mut e.element, &mut e.human),
            Self::Civilian(e) => apply(&mut e.element, &mut e.human),
            _ => false,
        }
    }

    pub fn npc_data_mut(&mut self) -> Option<&mut NpcData> {
        match self {
            Self::Soldier(e) => Some(&mut e.npc),
            Self::Civilian(e) => Some(&mut e.npc),
            _ => None,
        }
    }

    pub fn ai_actor_data_mut(&mut self) -> Option<&mut AiActorData> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut(),
            Self::Soldier(e) => Some(&mut e.npc.ai),
            Self::Civilian(e) => Some(&mut e.npc.ai),
            _ => None,
        }
    }

    pub fn soldier_data(&self) -> Option<&SoldierData> {
        match self {
            Self::Soldier(e) => Some(&e.soldier),
            _ => None,
        }
    }

    pub fn pc_data(&self) -> Option<&PcData> {
        match self {
            Self::Pc(e) => Some(&e.pc),
            _ => None,
        }
    }

    pub fn pc_data_mut(&mut self) -> Option<&mut PcData> {
        match self {
            Self::Pc(e) => Some(&mut e.pc),
            _ => None,
        }
    }

    pub fn object_data(&self) -> Option<&ObjectData> {
        match self {
            Self::Bonus(e) => Some(&e.object),
            Self::Scroll(e) => Some(&e.object),
            Self::Projectile(e) => Some(&e.object),
            Self::Net(e) => Some(&e.object),
            _ => None,
        }
    }

    pub fn object_data_mut(&mut self) -> Option<&mut ObjectData> {
        match self {
            Self::Bonus(e) => Some(&mut e.object),
            Self::Scroll(e) => Some(&mut e.object),
            Self::Projectile(e) => Some(&mut e.object),
            Self::Net(e) => Some(&mut e.object),
            _ => None,
        }
    }

    pub fn fx_data(&self) -> Option<&FxData> {
        match self {
            Self::Fx(e) => Some(&e.fx),
            Self::Target(e) => Some(&e.fx),
            _ => None,
        }
    }

    /// Whether this is an ordinary elevation-zero FX that belongs in the
    /// dedicated background-animation pass.
    ///
    /// This mirrors `RHEngine::AddElement`: only `IsFXBase()` elements at
    /// elevation zero are removed from the normal display-order lists.
    /// Targets are FX-derived but not FX-base, and mobile child sprites are
    /// managed by their mobile owner, so both remain in the sorted list.
    pub fn is_background_animation(&self) -> bool {
        matches!(
            self,
            Self::Fx(e)
                if e.fx.mobile_index.is_none() && e.element.position().z == 0.0
        )
    }

    /// For FX-kind entities (Fx / Target) the per-frame draw is
    /// suppressed when the player has disabled "Display Animations"
    /// in the graphics options, unless one of the override conditions
    /// holds:
    ///   - `force_display` is set on this FX,
    ///   - the FX is a patch animation (`patch_index.is_some()`),
    ///   - the FX is elevated (z != 0).
    ///
    /// Non-FX entities (PCs, NPCs, soldiers, civilians, …) always
    /// display — the check exists only on FX-base entities.
    pub fn is_to_be_displayed(&self, display_anim: bool) -> bool {
        let Some(fx) = self.fx_data() else {
            return true;
        };
        let elem = self.element_data();
        fx.force_display || fx.patch_index.is_some() || elem.position().z != 0.0 || display_anim
    }

    /// Return the display masking polyline for this entity.
    ///
    /// FX-base entities (Fx, Target) may have a polyline that
    /// determines whether other entities render in front of or behind
    /// them.  For Targets the polyline lives on `TargetData`; for Fx
    /// it lives on `FxData`.
    pub fn display_polyline(&self) -> &[MapPoint] {
        match self {
            Self::Target(e) => &e.target.display_polyline,
            Self::Fx(e) => &e.fx.display_polyline,
            _ => &[],
        }
    }

    // — Type checks (delegated to ElementKind) —

    pub fn is_actor(&self) -> bool {
        self.kind().is_actor()
    }
    pub fn is_human(&self) -> bool {
        self.kind().is_human()
    }
    pub fn is_pc(&self) -> bool {
        self.kind().is_pc()
    }
    pub fn is_npc(&self) -> bool {
        self.kind().is_npc()
    }
    pub fn is_soldier(&self) -> bool {
        self.kind().is_soldier()
    }
    pub fn is_civilian(&self) -> bool {
        self.kind().is_civilian()
    }
    pub fn is_fx(&self) -> bool {
        self.kind().is_fx()
    }
    pub fn is_fx_target(&self) -> bool {
        self.kind().is_fx_target()
    }
    pub fn is_object(&self) -> bool {
        self.kind().is_object()
    }
    pub fn is_projectile(&self) -> bool {
        self.kind().is_projectile()
    }
    pub fn is_bonus(&self) -> bool {
        self.kind().is_bonus()
    }

    /// Map-space anchor used for sprite placement and sprite-shaped UI
    /// affordances such as hit tests and hover outlines.
    ///
    /// Targets are special: their `position_map` is the action/interact
    /// point, while the visible sprite is authored at the preserved 3D
    /// `position` (`RHElementFX` draws the generated edge map from the
    /// same sprite position as the target sprite). Other actors keep using
    /// `position_map`, adjusted by jump height so airborne sprites and
    /// their outlines stay together.
    pub fn sprite_visual_map_position(&self) -> MapPoint {
        let elem = self.element_data();
        if self.is_fx_target() {
            elem.position().to_map()
        } else {
            let pos = elem.position_map();
            let jump_z = self.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
            MapPoint::new(pos.x, pos.y - jump_z)
        }
    }

    /// Camp allegiance for fighter-camp-keyed iteration. PCs, soldiers, and
    /// civilians read their authored cached camp. Non-actor entities have no
    /// camp and return `Camp::Error`.
    pub fn camp(&self) -> Camp {
        match self {
            Self::Pc(pc) => pc.pc.cached_camp,
            Self::Soldier(s) => s.soldier.cached_camp,
            Self::Civilian(c) => c.civilian.cached_camp,
            _ => Camp::Error,
        }
    }

    /// Build a [`crate::gate::ActorAuthInfo`] for door/gate authorization checks.
    ///
    /// Extracts the kind, lock-bypass abilities, rider status and posture
    /// from the entity.  For PCs the auth bit is `1 << profile_index`
    /// and lockpick/climb availability comes from `disabled_actions`.
    pub fn actor_auth_info(&self) -> crate::gate::ActorAuthInfo {
        let kind = self.kind();
        let posture = self.element_data().posture;

        // PC-specific fields
        let (pc_auth_bit, has_lockpick, has_climb, has_jump) = if let Some(pc) = self.pc_data() {
            let auth_bit = 1u16 << u32::from(pc.profile_index).min(15);
            (auth_bit, pc.has_lockpick, pc.has_climb, pc.has_jump)
        } else {
            (0, false, false, false)
        };

        let is_rider = self.soldier_data().map(|s| s.rider).unwrap_or(false);

        crate::gate::ActorAuthInfo {
            kind,
            pc_auth_bit,
            has_lockpick,
            has_climb,
            has_jump,
            is_rider,
            posture,
        }
    }

    // — Virtual method equivalents with per-type dispatch —

    pub fn is_immortal(&self) -> bool {
        match self {
            Self::Pc(e) => e.pc.immortal,
            _ => false,
        }
    }

    pub fn is_dead(&self) -> bool {
        match self {
            Self::Pc(e) => e.pc.life_points <= 0,
            Self::Soldier(e) => e.npc.life_points <= 0,
            Self::Civilian(e) => e.npc.life_points <= 0,
            _ => true,
        }
    }

    /// Live life points of a human element; `0` for everything else.
    pub fn human_life_points(&self) -> i16 {
        match self {
            Self::Pc(e) => e.pc.life_points,
            Self::Soldier(e) => e.npc.life_points,
            Self::Civilian(e) => e.npc.life_points,
            _ => 0,
        }
    }

    /// Maximum life points of a human element: the difficulty-scaled
    /// soldier-profile value for soldiers, a flat `100` for PCs and
    /// civilians, `0` for everything else.
    pub fn human_max_life_points(&self) -> i16 {
        match self {
            Self::Pc(_) | Self::Civilian(_) => 100,
            Self::Soldier(e) => e.soldier.cached_max_life_points,
            _ => 0,
        }
    }

    pub fn is_transporting(&self) -> bool {
        match self {
            Self::Pc(e) => e.element.posture == Posture::CarryingCorpse,
            _ => false,
        }
    }

    /// Door-transit half of "is inside a building": true while an
    /// actor is in the middle of a pass-door animation, before its
    /// sector pointer has been swapped to the inside-building sector.
    pub fn is_in_door_transit(&self) -> bool {
        !self.position_iface().get_door().is_null()
    }

    /// `IsInMotion` folded onto `Entity` so `&Entity` callers (e.g.
    /// the right-click handler) don't need to downcast to a concrete
    /// actor variant.  See [`Actor::is_in_motion`] for the semantic.
    pub fn is_in_motion(&self) -> bool {
        let pi = self.position_iface();
        let goal = pi.map_goal();
        let pos = pi.map_position();
        (goal != pos && goal != MapPoint::ZERO) || pi.is_moving_map()
    }

    // — Cross-module accessors —

    /// Get the entity's sprite.
    pub fn sprite(&self) -> &Sprite {
        &self.element_data().sprite
    }

    /// Get a mutable reference to the entity's sprite.
    pub fn sprite_mut(&mut self) -> &mut Sprite {
        &mut self.element_data_mut().sprite
    }

    /// Get the entity's position interface. Every entity has one (it's
    /// stored on the sprite).
    pub fn position_iface(&self) -> &PositionInterface {
        &self.element_data().sprite.position_iface
    }

    /// Get the entity's position interface mutably.
    pub fn position_iface_mut(&mut self) -> &mut PositionInterface {
        &mut self.element_data_mut().sprite.position_iface
    }

    /// C++ `GetPositionSprite()` equivalent for gameplay hotspot lookups.
    ///
    /// `Sprite::center` is the anchor loaded from the sprite data and used by
    /// rendering/input. C++ computes sprite position as
    /// `position_map - sprite_center`, so reconstruct that here. Targets keep
    /// their visible sprite anchor separate from their interaction point:
    /// `RHElementTarget::InitializeFromFile` computes the sprite position from
    /// the authored 3D placement, then overwrites only `PositionMap` with the
    /// action point and marks both values computed. `sprite_visual_map_position`
    /// preserves that distinction.
    pub fn cxx_position_sprite(&self) -> SpriteTopLeft {
        if self.is_fx_target()
            && let Some(position) = self.position_iface().cached_sprite_position()
        {
            return SpriteTopLeft::new(position.x, position.y);
        }
        let map = if self.is_fx_target() {
            self.sprite_visual_map_position()
        } else {
            self.element_data().position_map()
        };
        let center = self.sprite().center;
        SpriteTopLeft::new((map.x - center.x).floor(), (map.y - center.y).floor())
    }

    /// C++ `RHSprite::GetCurrentPointMap()` equivalent.
    ///
    /// Sprite-script hotspots are relative to the integer sprite top-left,
    /// not to the entity's map position.  Keeping that distinction matters
    /// for `RHMOVE_USE_POINT` seeks: the original compares and paths toward
    /// this map-space point.
    pub fn cxx_current_point_map(&self) -> Option<MapPoint> {
        let hotspot = self.sprite().current_hotspot()?;
        let sprite_position = self.cxx_position_sprite();
        Some(MapPoint::new(
            sprite_position.x + hotspot.x,
            sprite_position.y + hotspot.y,
        ))
    }

    /// Original `RHElement::GetPositionGround()` equivalent.
    ///
    /// The C++ method returns the X/Y components of the stored 3D position;
    /// it does not reconstruct them from `PositionMap` and the current plane.
    /// That distinction is observable when the two positions are deliberately
    /// independent and also avoids a second plane evaluation changing a
    /// direction-sector boundary.
    pub fn ground_position(&self) -> GroundPoint {
        let position = self.element_data().position();
        GroundPoint::new(position.x, position.y)
    }

    /// Get an actor's base AI controller, if it owns an AI brain.
    pub fn ai_controller(&self) -> Option<&AiController> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref()?.ai_brain.base(),
            Self::Soldier(e) => e.npc.ai_brain.base(),
            Self::Civilian(e) => e.npc.ai_brain.base(),
            _ => None,
        }
    }

    /// Get an actor's base AI controller mutably.
    pub fn ai_controller_mut(&mut self) -> Option<&mut AiController> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut()?.ai_brain.base_mut(),
            Self::Soldier(e) => e.npc.ai_brain.base_mut(),
            Self::Civilian(e) => e.npc.ai_brain.base_mut(),
            _ => None,
        }
    }

    /// Get the enemy AI subclass, if this actor has enemy AI.
    pub fn enemy_ai(&self) -> Option<&EnemyAi> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref()?.ai_brain.enemy(),
            Self::Soldier(e) => e.npc.ai_brain.enemy(),
            _ => None,
        }
    }

    /// Get the enemy AI subclass mutably.
    pub fn enemy_ai_mut(&mut self) -> Option<&mut EnemyAi> {
        match self {
            Self::Pc(e) => e.pc.ai.as_deref_mut()?.ai_brain.enemy_mut(),
            Self::Soldier(e) => e.npc.ai_brain.enemy_mut(),
            _ => None,
        }
    }

    /// Get the friendly AI subclass, if this actor has friendly AI.
    pub fn friendly_ai(&self) -> Option<&FriendlyAi> {
        match self {
            Self::Civilian(e) => e.npc.ai_brain.friendly(),
            _ => None,
        }
    }

    /// Get the friendly AI subclass mutably.
    pub fn friendly_ai_mut(&mut self) -> Option<&mut FriendlyAi> {
        match self {
            Self::Civilian(e) => e.npc.ai_brain.friendly_mut(),
            _ => None,
        }
    }

    /// Compute the 3D eye point of a Human actor (PC / soldier / civilian).
    ///
    /// Used by the shadow polygon / view cone overlay: the overlay
    /// only renders when `eye.z >= 0`, which filters out dead or
    /// teleported-away characters.
    ///
    /// `override_posture`: if `Some`, use this posture instead of the
    /// entity's current one.  Default (None) reads `element.posture`.
    ///
    /// Returns `None` for non-Human entities (FX, objects).
    pub fn compute_eyes_point(&self, override_posture: Option<Posture>) -> Option<WorldPoint3D> {
        // Only Human actors have posture-dependent eye offsets.
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        // Rider flag — only mounted soldiers ride.
        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);

        // Emergency-lying-box halves crawling offsets.
        let emergency_lying = self.position_iface().is_using_emergency_lying_box();

        // The authoritative ground position lives in
        // `element.position_map`; see `human_feet_point_3d`.
        let mut eyes = self.human_feet_point_3d();
        // `element.posture` is not initialised at entity load (only
        // combat / ability code writes to it), so we treat
        // `Undefined` as `Upright` here to match the normal human
        // resting state.
        let raw_posture = override_posture.unwrap_or(e.posture);
        let posture = if raw_posture == Posture::Undefined {
            Posture::Upright
        } else {
            raw_posture
        };
        let dir = (e.direction().rem_euclid(16)) as usize;

        use Posture::*;
        match posture {
            HelpingToClimb | CarryingOnShoulders | Upright | OnLadder | OnWall | Flying
            | CarryingCorpse | Leisure | Spy | AnonymousArcher | Siesta => {
                eyes.z += if is_rider { 60.0 } else { 45.0 };
            }
            OnShoulders => {
                eyes.z += 85.0;
            }
            Crouched | Sitting | SimulatingBeggar | Tree => {
                eyes.z += 25.0;
            }
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                let scale = if emergency_lying { 0.5 } else { 1.0 };
                eyes.x += scale * CRAWLING_OFFSETS_X[dir];
                eyes.y += scale * CRAWLING_OFFSETS_Y[dir];
                eyes.z += 5.0;
            }
            LeaningOut => {
                // Bend forward by 40 units along the facing direction.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                eyes.x += dx * 40.0;
                eyes.y += dy * 40.0;
                eyes.z += 45.0;
            }
            // Carried / Unused / Undefined — return the feet position
            // with a small offset so the overlay still works.
            _ => {
                eyes.z += 25.0;
            }
        }

        Some(eyes)
    }

    /// Compute the detection point of a human actor.
    ///
    /// This is the *target side* 3D point used by NPC
    /// `compute_visibility`.
    ///
    /// Differs from [`compute_eyes_point`]:
    /// - Lying / Dead / DeadBack / StuckUnderNet / Tied: z+2, no
    ///   `crawlingOffsets` lateral shift (eyes uses z+5 with shift).
    /// - Carried: enumerated at z+25 (eyes asserts).
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_detection_point(&self) -> Option<WorldPoint3D> {
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);

        // Original's ComputeDetectionPoint copies GetPosition(), the raw
        // retained 3-D cache. During the bounded elevation-crossing callback
        // window that cache still names the outgoing plane; resolving it
        // here exposes the incoming plane one callback too early.
        let mut pt = self.element_data().position();
        let raw_posture = e.posture;
        let posture = if raw_posture == Posture::Undefined {
            Posture::Upright
        } else {
            raw_posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                pt.z += if is_rider { 60.0 } else { 45.0 };
            }
            LeaningOut => {
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                pt.x += dx * 40.0;
                pt.y += dy * 40.0;
                pt.z += 45.0;
            }
            OnShoulders => {
                pt.z += 85.0;
            }
            Crouched | Sitting | SimulatingBeggar | Tree | Carried => {
                pt.z += 25.0;
            }
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                pt.z += 2.0;
            }
            // Unused / Undefined — mirror eyes-point's permissive
            // fallback so callers don't crash on unset postures during
            // entity load.
            _ => {
                pt.z += 25.0;
            }
        }

        Some(pt)
    }

    /// Compute the position for star titbits above a human actor.
    ///
    /// Differs from [`compute_eyes_point`] for specific postures:
    /// - **Dead/DeadBack**: offset 30 units along/against facing direction
    /// - **Carried**: half-crawling offset, z+32
    /// - **LeaningOut**: offset 10 units along facing direction
    /// - **Rider**: offset -10 along direction, z+65
    /// - **Default**: falls through to `compute_eyes_point`
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_stars_point(&self) -> Option<WorldPoint3D> {
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        // Live feet point — see note on `human_feet_point_3d`.
        let base = self.human_feet_point_3d();

        // Rider: offset backward from facing direction, high Z.
        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        if is_rider {
            let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
            return Some(WorldPoint3D {
                x: base.x - dx * 10.0,
                y: base.y - dy * 10.0,
                z: base.z + 65.0,
            });
        }

        use Posture::*;
        match e.posture {
            Lying | StuckUnderNet | Tied => {
                //   pt_map = floor(position_map - sprite.center) + sprite_hotspot
                //   pt_stars = (pt_map.x, pt_map.y + elev+5, elev+5)
                // The sprite-position floor is applied before the row hotspot
                // is added, so the titbit anchor matches the rendered
                // body.
                //
                // Y carries `elevation + 5` to match the codebase-wide
                // iso-Y invariant (`y = map.y + z`) — same as every
                // other arm built off `human_feet_point_3d`.
                let sprite = self.sprite();
                let map = self.element_data().position_map();
                let center = sprite.center;
                let hp = sprite.hotspot_for_row(sprite.current_row);
                let elevation = base.z;
                Some(WorldPoint3D {
                    x: (map.x - center.x).floor() + hp.x,
                    y: (map.y - center.y).floor() + hp.y + elevation + 5.0,
                    z: elevation + 5.0,
                })
            }
            Dead => {
                // Head fell forward — offset 30 units along facing.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x + dx * 30.0,
                    y: base.y + dy * 30.0,
                    z: base.z + 5.0,
                })
            }
            DeadBack => {
                // Fell backward — offset 30 units against facing.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x - dx * 30.0,
                    y: base.y - dy * 30.0,
                    z: base.z + 5.0,
                })
            }
            Carried => {
                // Flip direction by 180° (`(dir + 8) & 15`) when the
                // current animation is `BeingCarriedLittleJohn` or
                // `BeingCarriedPeasantC`.  `Posture::Carried` is only
                // set after the lift transition completes, so when
                // posture is Carried the animation is always one of
                // those two and we apply the flip unconditionally.
                let flipped_dir = ((e.direction().wrapping_add(8)) & 15) as usize;
                Some(WorldPoint3D {
                    x: base.x + 0.5 * CRAWLING_OFFSETS_X[flipped_dir],
                    y: base.y + 0.5 * CRAWLING_OFFSETS_Y[flipped_dir],
                    z: base.z + 32.0,
                })
            }
            LeaningOut => {
                // Leaning forward out of a window — small forward offset.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(e.direction());
                Some(WorldPoint3D {
                    x: base.x + dx * 10.0,
                    y: base.y + dy * 10.0,
                    z: base.z + 45.0,
                })
            }
            // All other postures use the general eyes point.
            _ => self.compute_eyes_point(None),
        }
    }

    /// Compute the belt point (centre of mass) of a human actor.
    ///
    /// Returns `None` for non-Human entities (FX, objects).
    pub fn compute_belt_point(&self) -> Option<WorldPoint3D> {
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Original's ComputeBeltPoint copies RHPositionInterface::GetPosition,
        // whose release-build accessor returns the retained mpointPosition
        // bytes without forcing ComputePosition3D. This matters during the
        // bounded elevation-crossing callback window: an arrow released there
        // must aim from the outgoing cached plane, just like the shipped game.
        let mut belt = self.element_data().position();
        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | LeaningOut | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                belt.z += if is_rider {
                    RIDER_ELEVATION_BELT_UPRIGHT
                } else {
                    HUMAN_ELEVATION_BELT_UPRIGHT
                };
            }
            OnShoulders => belt.z += 65.0,
            Carried => belt.z += 55.0,
            Sitting | Crouched | SimulatingBeggar | Tree => belt.z += 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => belt.z += 5.0,
            // Unknown postures get a safe fallback.
            _ => belt.z += 10.0,
        }

        Some(belt)
    }

    /// Live base 3D point every `compute_*_point` posture switch starts from:
    /// the actor's stored 3D position.
    ///
    /// It satisfies `y = map.y + z`, but only the stored value is the numeric
    /// contract. Recomputing it as `map.y + elevation` lands one bit away
    /// whenever the map coordinates were themselves projected down from a
    /// 3D-authoritative position, which then shows up in the endpoints of
    /// opaque-reachability queries.
    fn human_feet_point_3d(&self) -> WorldPoint3D {
        self.element_data().position()
    }

    /// Compute the hand point of a human actor.
    ///
    /// Per-frame sprite hand-anchor for the current animation row,
    /// with `+elevation` folded into Y.
    ///
    /// `forced_elevation`: when `Some(value)`, the Z is set to
    /// `elevation + value` and the posture switch is skipped.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_hand_point(&self, forced_elevation: Option<f32>) -> Option<WorldPoint3D> {
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Seed X/Y from the per-frame sprite hotspot; fall back to
        // the feet point if the sprite has no script bound (headless
        // test).
        let elevation = self.position_iface().get_elevation();
        let mut hand = match self.sprite().current_hotspot() {
            Some(hp) => {
                let ps = self.cxx_position_sprite();
                WorldPoint3D {
                    x: ps.x + hp.x,
                    y: ps.y + hp.y + elevation,
                    z: elevation,
                }
            }
            None => self.human_feet_point_3d(),
        };

        // When `forced_elevation` is provided, skip the posture switch.
        if let Some(fe) = forced_elevation {
            hand.z = elevation + fe;
            return Some(hand);
        }

        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                hand.z = elevation
                    + if is_rider {
                        45.0
                    } else {
                        HUMAN_ELEVATION_BELT_UPRIGHT
                    };
            }
            LeaningOut => hand.z = elevation + 25.0,
            OnShoulders => hand.z = elevation + 65.0,
            Sitting | Crouched | SimulatingBeggar | Tree => hand.z = elevation + 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => hand.z = elevation + 5.0,
            // Unknown postures get a safe fallback.
            _ => hand.z = elevation + 10.0,
        }

        Some(hand)
    }

    /// Compute the hand point with explicit direction, animation, and posture.
    ///
    /// Used for throwing projectiles where the animation and facing
    /// direction may differ from the entity's current state.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_hand_point_for_posture(
        &self,
        direction: i16,
        animation: OrderType,
        posture: Posture,
    ) -> Option<WorldPoint3D> {
        // Validate this is a human entity.
        match self {
            Self::Pc(_) | Self::Soldier(_) | Self::Civilian(_) => {}
            _ => return None,
        }

        let is_rider = matches!(self, Self::Soldier(s) if s.soldier.rider);
        // Seed X/Y from the sprite hotspot for the requested
        // animation+direction (mirrors bow_shot::shoot_order_type_for_mode
        // sprite lookup pattern).  Fall back to feet point if the lookup
        // fails (e.g. unmapped animation).
        let elevation = self.position_iface().get_elevation();
        let mut hand = match self.sprite().get_point(animation, direction as u16) {
            Some(hp) => {
                let ps = self.cxx_position_sprite();
                WorldPoint3D {
                    x: ps.x + hp.x,
                    y: ps.y + hp.y + elevation,
                    z: elevation,
                }
            }
            None => self.human_feet_point_3d(),
        };
        let posture = if posture == Posture::Undefined {
            Posture::Upright
        } else {
            posture
        };

        use Posture::*;
        match posture {
            Upright | Spy | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                hand.z = elevation
                    + if is_rider {
                        40.0
                    } else {
                        HUMAN_ELEVATION_BELT_UPRIGHT
                    };
            }
            LeaningOut => hand.z = elevation + 25.0,
            OnShoulders => hand.z = elevation + 65.0,
            Sitting | Crouched | SimulatingBeggar | Tree => hand.z = elevation + 10.0,
            Lying | Dead | DeadBack | StuckUnderNet | Tied => hand.z = elevation + 5.0,
            _ => hand.z = elevation + 10.0,
        }

        Some(hand)
    }

    /// Compute the feet point of a human actor.
    ///
    /// Used by the Lock titbit kind to display at foot level.
    ///
    /// Returns `None` for non-Human entities.
    pub fn compute_feet_point(&self) -> Option<WorldPoint3D> {
        let (e, _) = match self {
            Self::Pc(e) => (&e.element, true),
            Self::Soldier(e) => (&e.element, true),
            Self::Civilian(e) => (&e.element, true),
            _ => return None,
        };

        let emergency_lying = self.position_iface().is_using_emergency_lying_box();

        let mut feet = self.human_feet_point_3d();
        let posture = if e.posture == Posture::Undefined {
            Posture::Upright
        } else {
            e.posture
        };

        use Posture::*;
        match posture {
            // Standing postures: feet at ground level + 5.
            Upright | Spy | LeaningOut | Leisure | Siesta | CarryingCorpse | HelpingToClimb
            | CarryingOnShoulders | AnonymousArcher | OnLadder | OnWall | Flying => {
                feet.z += 5.0;
            }
            // Crouching/sitting: same small offset.
            Tree | Crouched | Sitting | SimulatingBeggar => {
                feet.z += 5.0;
            }
            // On shoulders / carried: elevated.
            OnShoulders | Carried => {
                feet.z += 45.0;
            }
            // Lying/dead: feet displaced opposite to facing direction.
            Lying | Dead | DeadBack | StuckUnderNet | Tied => {
                let dir = (e.direction().rem_euclid(16)) as usize;
                let scale = if emergency_lying { 0.5 } else { 1.0 };
                feet.x -= scale * CRAWLING_OFFSETS_X[dir];
                feet.y -= scale * CRAWLING_OFFSETS_Y[dir];
                feet.z += 5.0;
            }
            _ => {
                feet.z += 5.0;
            }
        }

        Some(feet)
    }

    /// 3D centre point used as the projectile aim anchor for FX targets.
    ///
    /// Start from the element position and lift `z` by half the
    /// sprite's screen-Y extent.  In this isometric projection
    /// screen-Y and world-Z run along the same visual axis, so the
    /// half-pixel-height add lands the centre roughly midway up the
    /// sprite.
    ///
    /// Returns `None` for non-FX-target entities — those have their own
    /// dedicated centre helpers (`compute_eyes_point`, `compute_hand_point`,
    /// etc.) that the caller should reach for instead.
    pub fn compute_target_center(&self) -> Option<WorldPoint3D> {
        if !self.is_fx_target() {
            return None;
        }
        let elem = self.element_data();
        let half_h = elem.sprite.current_max_height() as f32 * 0.5;
        Some(WorldPoint3D {
            x: elem.position().x,
            y: elem.position().y,
            z: elem.position().z + half_h,
        })
    }

    /// Get the AI top-level state for NPCs.
    pub fn ai_state(&self) -> Option<AiTopState> {
        match self {
            Self::Soldier(e) => Some(e.npc.ai_state()),
            Self::Civilian(e) => Some(e.npc.ai_state()),
            _ => None,
        }
    }
}

/// Concrete Original class selected by Rust's entity/object discriminants.
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

/// Base trait for all game entities.
pub trait Element {
    fn element_data(&self) -> &ElementData;
    fn element_data_mut(&mut self) -> &mut ElementData;

    fn kind(&self) -> ElementKind {
        self.element_data().kind
    }
    fn is_active(&self) -> bool {
        self.element_data().active
    }
    fn is_blipped(&self) -> bool {
        self.element_data().blipped
    }
    fn is_unreachable(&self) -> bool {
        self.element_data().unreachable
    }
    #[must_use = "method returns WorldPoint3D by value; `elem.position().x = v` silently modifies a temporary. Use `set_position` to mutate."]
    fn position(&self) -> WorldPoint3D {
        self.element_data().position()
    }
    #[must_use = "method returns MapPoint by value; `elem.position_map().x = v` silently modifies a temporary. Use `set_position_map` to mutate."]
    fn position_map(&self) -> MapPoint {
        self.element_data().position_map()
    }
    #[must_use]
    fn direction(&self) -> i16 {
        self.element_data().direction()
    }
    fn posture(&self) -> Posture {
        self.element_data().posture
    }
    /// Set posture through the corpse-transition guard.  Delegates to
    /// [`ElementData::set_posture`].
    fn set_posture(&mut self, p: Posture) {
        self.element_data_mut().set_posture(p);
    }
    fn class_id(&self) -> u16 {
        self.element_data().class_id
    }
    fn is_in_honolulu(&self) -> bool {
        self.element_data().in_honolulu
    }

    // — Virtual methods with default impls —
    fn is_immortal(&self) -> bool {
        false
    }
    fn is_engine_locked(&self) -> bool {
        false
    }
    fn is_transporting(&self) -> bool {
        false
    }
    fn is_dead(&self) -> bool {
        true
    }
    fn is_obviously_hostile(&self) -> bool {
        false
    }

    /// Per-frame update tick. Returns false if the entity should be removed.
    fn hourglass(&mut self) -> bool {
        true
    }
}

/// Trait for actor entities.
pub trait Actor: Element {
    fn actor_data(&self) -> &ActorData;
    fn actor_data_mut(&mut self) -> &mut ActorData;

    fn action_state(&self) -> ActionState {
        self.actor_data().action_state
    }
    fn wait_time(&self) -> u32 {
        self.actor_data().wait_time
    }
    fn is_execution_frozen(&self) -> bool {
        self.actor_data().execution_frozen
    }
    fn is_tied(&self) -> bool {
        self.posture() == Posture::Tied
    }
    /// `IsInMotion` folded into the base trait since every caller is
    /// a PC/human and the human override strictly subsumes the actor
    /// version.  Returns true if the sprite is translating toward a
    /// non-zero goal OR has actually moved on the map between
    /// frames.
    fn is_in_motion(&self) -> bool {
        let pi = &self.element_data().sprite.position_iface;
        let goal = pi.map_goal();
        let pos = pi.map_position();
        (goal != pos && goal != MapPoint::ZERO) || pi.is_moving_map()
    }
    fn is_ignored(&self) -> bool {
        false
    }
}

/// Trait for human actors.
pub trait Human: Actor {
    fn human_data(&self) -> &HumanData;
    fn human_data_mut(&mut self) -> &mut HumanData;

    /// Life point source — must be provided by each concrete type.
    fn life_points(&self) -> i16;
    fn max_life_points(&self) -> i16;
    fn camp(&self) -> Camp;

    fn is_unconscious(&self) -> bool {
        self.human_data().unconscious
    }
    /// OR-combine `IsDead` / `IsUnconscious` / `IsStuckUnderNet` /
    /// `posture == Tied` / `posture == Carried` /
    /// `IsPC() && IsInComa()`.
    ///
    /// The `IsInComa` branch isn't reachable here because `in_coma`
    /// lives on `Campaign` (`PcStatus`), not on `PcData` — callers
    /// that need the coma arm must compose it externally (see
    /// `ai_entity_view.rs` for the campaign-aware path).
    fn is_out_of_order(&self) -> bool {
        self.life_points() <= 0
            || self.is_unconscious()
            || self.is_stuck_under_net()
            || matches!(self.posture(), Posture::Tied | Posture::Carried)
    }
    fn concussion(&self) -> u16 {
        self.human_data().concussion_of_the_brain
    }
    fn is_invulnerable(&self) -> bool {
        self.human_data().invulnerable
    }
    fn is_carried(&self) -> bool {
        self.human_data().carrier.is_some()
    }
    fn is_stuck_under_net(&self) -> bool {
        self.human_data().stuck_under_nets_counter > 0
    }
    fn is_hollow_man(&self) -> bool {
        self.human_data().hollow_man
    }
    fn is_killed_by_accident(&self) -> bool {
        self.human_data().killed_by_accident
    }
    fn is_holding_shield(&self) -> bool {
        self.action_state().is_shield()
    }
    fn tiredness(&self) -> u16 {
        self.human_data().tiredness
    }
    fn is_enemy_of(&self, other_camp: Camp) -> bool {
        self.camp().is_hostile_to(other_camp)
    }
    fn enemy_camp(&self) -> Camp {
        // TODO(multi-team-diplomacy): remove this binary compatibility
        // accessor once remaining callers request concrete hostile actors.
        match self.camp() {
            Camp::Royalists => Camp::Lacklandists,
            Camp::Lacklandists => Camp::Royalists,
            Camp::Custom(id) => {
                panic!("custom allegiance {id} has multiple possible enemy camps; use is_enemy_of")
            }
            Camp::Error => panic!("invalid camp has no enemy camp"),
        }
    }
    fn is_robin(&self) -> bool {
        false
    }
    fn is_able_to_fight(&self) -> bool {
        false
    }
    fn is_able_to_help(&self) -> bool {
        false
    }
    fn fighting_ability(&self) -> u16 {
        0
    }
    fn shooting_ability(&self) -> u16 {
        0
    }
    fn endurance(&self) -> u16 {
        0
    }

    /// Whether this human is a credible menacer of `prisoner_pos`.
    ///
    /// Gates on the current `action_state` being one of `Waiting` /
    /// `AimingWithBow` / `Moving` / `MovingFast`, then dot-products
    /// the prisoner offset against the menacer's facing direction
    /// (positive = in front).
    ///
    /// No in-tree callers yet (this is vestigial); the body is
    /// provided so the first caller gets the right behaviour rather
    /// than the ad-hoc `true`/`false` defaults that used to live on
    /// `Element`/`ActorSoldier`.
    fn is_dangerous_as_menacer(&self, prisoner_pos: MapPoint) -> bool {
        match self.action_state() {
            ActionState::Waiting
            | ActionState::AimingWithBow
            | ActionState::Moving
            | ActionState::MovingFast => {}
            _ => return false,
        }
        let here = self.position_map();
        let dx = prisoner_pos.x - here.x;
        let dy = prisoner_pos.y - here.y;
        let [vx, vy] = crate::position_interface::sector_to_vector_iso(self.direction());
        dx * vx + dy * vy > 0.0
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Trait implementations
// ═══════════════════════════════════════════════════════════════════

macro_rules! impl_element_data {
    ($ty:ty) => {
        impl crate::element::Element for $ty {
            fn element_data(&self) -> &ElementData {
                &self.element
            }
            fn element_data_mut(&mut self) -> &mut ElementData {
                &mut self.element
            }
        }
    };
}

macro_rules! impl_actor_data {
    ($ty:ty) => {
        impl crate::element::Actor for $ty {
            fn actor_data(&self) -> &ActorData {
                &self.actor
            }
            fn actor_data_mut(&mut self) -> &mut ActorData {
                &mut self.actor
            }
        }
    };
}

// -- Element trait for all concrete types --

impl Element for ActorPc {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_immortal(&self) -> bool {
        self.pc.immortal
    }
    fn is_transporting(&self) -> bool {
        self.posture() == Posture::CarryingCorpse
    }
    fn is_dead(&self) -> bool {
        self.pc.life_points <= 0
    }
    fn is_obviously_hostile(&self) -> bool {
        true
    }
}

impl Element for ActorSoldier {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_dead(&self) -> bool {
        self.npc.life_points <= 0
    }
}

impl Element for ActorCivilian {
    fn element_data(&self) -> &ElementData {
        &self.element
    }
    fn element_data_mut(&mut self) -> &mut ElementData {
        &mut self.element
    }
    fn is_dead(&self) -> bool {
        self.npc.life_points <= 0
    }
}

impl_element_data!(ElementFx);
impl_element_data!(ElementTarget);
impl_element_data!(ElementBonus);
impl_element_data!(ElementProjectile);
impl_element_data!(ElementNet);

// -- Actor trait for actor types --

impl_actor_data!(ActorPc);
impl_actor_data!(ActorSoldier);
impl_actor_data!(ActorCivilian);

// -- Human trait for human types --

impl Human for ActorPc {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.pc.life_points
    }
    fn max_life_points(&self) -> i16 {
        100
    }
    fn camp(&self) -> Camp {
        self.pc.cached_camp
    }
    fn is_robin(&self) -> bool {
        self.pc.robin
    }
    /// Guards on dead/unconscious/inactive, then returns false for
    /// disguised postures (`Tree`, `Spy`).
    fn is_able_to_fight(&self) -> bool {
        if self.pc.life_points <= 0 || self.is_unconscious() || !self.is_active() {
            return false;
        }
        !matches!(self.posture(), Posture::Tree | Posture::Spy)
    }
}

impl Human for ActorSoldier {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.npc.life_points
    }
    fn max_life_points(&self) -> i16 {
        // Level initialization already applies the difficulty modifier when
        // populating this cache. Scaling again here inflated Lacklandist HP.
        self.soldier.cached_max_life_points
    }
    fn camp(&self) -> Camp {
        self.soldier.cached_camp
    }
    /// Two layers: an early-false block (dead / unconscious / tied /
    /// carried / inactive), then a state-machine switch where
    /// `Sleeping`, `Menacing`, `Fleeing`, and the three hit-stun
    /// `Attacking` substates all return false.
    fn is_able_to_fight(&self) -> bool {
        if self.npc.life_points <= 0
            || self.is_unconscious()
            || self.is_tied()
            || self.is_carried()
            || !self.is_active()
        {
            return false;
        }
        match self.npc.ai_state() {
            AiTopState::Sleeping | AiTopState::Menacing | AiTopState::Fleeing => false,
            AiTopState::Default | AiTopState::Wondering | AiTopState::Seeking => true,
            AiTopState::Attacking => !matches!(
                self.npc.ai_substate(),
                AiSubstate::AttackingGotHit
                    | AiSubstate::AttackingGotHitStandingUp
                    | AiSubstate::AttackingHitting,
            ),
        }
    }
    /// Reject dead/unconscious, then state-machine:
    /// Default/Wondering → true; Seeking restricted to officer-report
    /// and reaction-time substates; Sleeping/Menacing/Fleeing/Attacking
    /// → false. Note: unlike `is_able_to_fight`, this predicate does
    /// NOT gate on tied/carried/inactive.
    fn is_able_to_help(&self) -> bool {
        if self.npc.life_points <= 0 || self.is_unconscious() {
            return false;
        }
        match self.npc.ai_state() {
            AiTopState::Default | AiTopState::Wondering => true,
            AiTopState::Seeking => matches!(
                self.npc.ai_substate(),
                AiSubstate::SeekingSoldierGiveReportToOfficer
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerStart
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerPoint
                    | AiSubstate::SeekingSoldierGiveAlertingReportToOfficerEnd
                    | AiSubstate::SeekingRunningToOfficer
                    | AiSubstate::SeekingRunningToOfficerSeen
                    | AiSubstate::SeekingHeardstepsReactiontime
                    | AiSubstate::SeekingBodyReactiontime
            ),
            AiTopState::Sleeping
            | AiTopState::Menacing
            | AiTopState::Fleeing
            | AiTopState::Attacking => false,
        }
    }
}

impl Human for ActorCivilian {
    fn human_data(&self) -> &HumanData {
        &self.human
    }
    fn human_data_mut(&mut self) -> &mut HumanData {
        &mut self.human
    }
    fn life_points(&self) -> i16 {
        self.npc.life_points
    }
    fn max_life_points(&self) -> i16 {
        100
    } // civilians always 100
    fn camp(&self) -> Camp {
        self.civilian.cached_camp
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Concrete-type convenience methods
// ═══════════════════════════════════════════════════════════════════

impl ActorPc {}

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
    /// Original concrete class represented by this Rust `Entity::Bonus`.
    ///
    /// Rust deliberately shares one storage variant for several Original
    /// object subclasses, so virtual dispatch must use the authored object
    /// type rather than assuming every value is `RHElementBonus`.
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
    /// TODO(original-parity): add a mapping only when an Original constructor
    /// proves that the object type can inhabit Rust's `Entity::Bonus`.
    Unsupported,
}

// ─── BonusItemType / ObjectType → Action bridges ──────────────────
//
// The enums live in `robin_engine::element_kinds`; inherent impls here
// would violate the orphan rule, and they can't be in the engine crate
// because `Action` is in `crate::profiles`. Extension traits bridge it.

pub trait BonusItemTypeExt {
    fn to_action(self) -> Action;
}

impl BonusItemTypeExt for BonusItemType {
    fn to_action(self) -> Action {
        match self {
            Self::Arrow => Action::Bow,
            Self::Stone => Action::Stone,
            Self::Apple => Action::Apple,
            Self::Ale => Action::Ale,
            Self::Lamb => Action::Eat,
            Self::Plant => Action::Heal,
            Self::Net => Action::Net,
            Self::WaspNest => Action::WaspNest,
            Self::Purse => Action::Purse,
            Self::Ransom
            | Self::Amulet
            | Self::Blazon
            | Self::Ampulla
            | Self::CoronationSpoon
            | Self::RichardsCrown
            | Self::RoyalSeal
            | Self::RoyalSceptre
            | Self::DomesdayBook
            | Self::SwordOfTheState => Action::NoAction,
        }
    }
}

pub trait ObjectTypeExt {
    fn to_action(self) -> Action;
}

impl ObjectTypeExt for ObjectType {
    fn to_action(self) -> Action {
        match self {
            Self::Arrow | Self::BonusArrow => Action::Bow,
            Self::Stone | Self::BonusStone => Action::Stone,
            Self::Apple | Self::BonusApple => Action::Apple,
            Self::Ale | Self::BonusAle => Action::Ale,
            Self::BonusLambLeg => Action::Eat,
            Self::BonusPlants => Action::Heal,
            Self::Net | Self::BonusNet => Action::Net,
            Self::WaspNest | Self::BonusWaspNest => Action::WaspNest,
            Self::Purse | Self::BonusPurse => Action::Purse,
            _ => Action::NoAction,
        }
    }
}

impl ElementProjectile {
    /// Run the movement portion of `RHElementProjectile::Hourglass`.
    /// Every virtual call snapshots `old_position` first; keeping that edge
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
    /// (the coin `HitObstacle` runs `Hourglass` once before
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
mod tests {
    use super::*;

    #[test]
    fn belt_point_uses_retained_position_during_elevation_crossing_callback() {
        use crate::position_interface::{ObstacleHandle, PlaneZCoeffs};

        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let position = pc.position_iface_mut();
        position.set_obstacle(
            Some(ObstacleHandle::new(1).unwrap()),
            Some(PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 36.0,
            }),
        );
        position.set_map_position(MapPoint::new(198.0, 282.0));
        let outgoing = position.get_position();
        position.set_obstacle(
            Some(ObstacleHandle::new(2).unwrap()),
            Some(PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 35.5,
            }),
        );
        position.restore_cached_position_3d_invalid(outgoing);

        assert_eq!(
            pc.compute_belt_point(),
            Some(WorldPoint3D::new(198.0, 318.0, 61.0)),
            "ComputeBeltPoint must preserve Original's raw GetPosition bytes"
        );
    }

    #[test]
    fn detection_point_uses_retained_position_during_elevation_crossing_callback() {
        use crate::position_interface::{ObstacleHandle, PlaneZCoeffs};

        let mut soldier = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData {
                rider: true,
                ..SoldierData::default()
            },
        });
        let position = soldier.position_iface_mut();
        position.set_obstacle(
            Some(ObstacleHandle::new(1).unwrap()),
            Some(PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 36.0,
            }),
        );
        position.set_map_position(MapPoint::new(198.0, 282.0));
        let outgoing = position.get_position();
        position.set_obstacle(
            Some(ObstacleHandle::new(2).unwrap()),
            Some(PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 35.5,
            }),
        );
        position.restore_cached_position_3d_invalid(outgoing);

        assert_eq!(
            soldier.compute_detection_point(),
            Some(WorldPoint3D::new(198.0, 318.0, 96.0)),
            "ComputeDetectionPoint must preserve Original's raw GetPosition bytes"
        );
    }

    #[test]
    fn delayed_positions_preserve_original_map_then_world_priority() {
        let mut element = ElementData::default();
        element.set_position(WorldPoint3D::new(10.0, 20.0, 3.0));
        element.set_position_map_delayed(MapPoint::new(40.0, 50.0));
        element.set_position_delayed(WorldPoint3D::new(70.0, 80.0, 9.0));

        let (old_map, new_map, _) = element
            .apply_next_delayed_position()
            .expect("map queue should apply first");
        assert_eq!(old_map, MapPoint::from_world_xyz(10.0, 20.0, 3.0));
        assert_eq!(new_map, MapPoint::new(40.0, 50.0));
        assert!(!element.position_map_delayed);
        assert!(element.position_delayed);
        assert_eq!(element.delayed_map_position, MapPoint::new(40.0, 50.0));

        let (_, second_map, _) = element
            .apply_next_delayed_position()
            .expect("world queue should remain for the next frame");
        assert_eq!(element.position(), WorldPoint3D::new(70.0, 80.0, 9.0));
        assert_eq!(second_map, MapPoint::from_world_xyz(70.0, 80.0, 9.0));
        assert!(!element.position_delayed);
        assert_eq!(
            element.delayed_position,
            WorldPoint3D::new(70.0, 80.0, 9.0),
            "Original clears only the flag and retains the serialized point"
        );
    }

    fn object_data(object_type: ObjectType) -> ObjectData {
        ObjectData {
            object_type,
            ..Default::default()
        }
    }

    fn element_data(kind: ElementKind) -> ElementData {
        ElementData {
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn original_hourglass_mapping_covers_every_rust_concrete_class() {
        use OriginalHourglassClass as Class;

        let cases = [
            (
                Entity::Pc(ActorPc {
                    element: element_data(ElementKind::ActorPc),
                    actor: ActorData::default(),
                    human: HumanData::default(),
                    pc: PcData::default(),
                }),
                Class::ActorPc,
            ),
            (
                Entity::Soldier(ActorSoldier {
                    element: element_data(ElementKind::ActorSoldier),
                    actor: ActorData::default(),
                    human: HumanData::default(),
                    npc: NpcData::default(),
                    soldier: SoldierData::default(),
                }),
                Class::ActorSoldier,
            ),
            (
                Entity::Civilian(ActorCivilian {
                    element: element_data(ElementKind::ActorCivilian),
                    actor: ActorData::default(),
                    human: HumanData::default(),
                    npc: NpcData::default(),
                    civilian: CivilianData::default(),
                }),
                Class::ActorCivilian,
            ),
            (
                Entity::Fx(ElementFx {
                    element: element_data(ElementKind::Fx),
                    fx: FxData::default(),
                }),
                Class::Fx,
            ),
            (
                Entity::Fx(ElementFx {
                    element: element_data(ElementKind::Fx),
                    fx: FxData {
                        mobile_index: Some(0),
                        ..Default::default()
                    },
                }),
                Class::FxMasked,
            ),
            (
                Entity::Target(ElementTarget {
                    element: element_data(ElementKind::Target),
                    fx: FxData::default(),
                    target: TargetData::default(),
                }),
                Class::Target,
            ),
            (
                Entity::Bonus(ElementBonus {
                    element: element_data(ElementKind::ObjectBonus),
                    object: object_data(ObjectType::BonusAmulet),
                }),
                Class::Bonus,
            ),
            (
                Entity::Bonus(ElementBonus {
                    element: element_data(ElementKind::ObjectOther),
                    object: object_data(ObjectType::Ale),
                }),
                Class::Ale,
            ),
            (
                Entity::Bonus(ElementBonus {
                    element: element_data(ElementKind::ObjectBonus),
                    object: object_data(ObjectType::Cape),
                }),
                Class::Cape,
            ),
            (
                Entity::Scroll(ElementScroll {
                    element: element_data(ElementKind::ObjectScroll),
                    object: object_data(ObjectType::Scroll),
                    ..Default::default()
                }),
                Class::Scroll,
            ),
        ];
        for (entity, expected) in cases {
            assert_eq!(entity.original_hourglass_class(), expected);
        }

        let projectile_cases = [
            (ObjectType::Arrow, Class::Arrow),
            (ObjectType::Apple, Class::Apple),
            (ObjectType::Stone, Class::Stone),
            (ObjectType::Purse, Class::Purse),
            (ObjectType::Coin, Class::Coin),
            (ObjectType::WaspNest, Class::WaspNest),
            (ObjectType::BonusWaspNest, Class::WaspNest),
            (ObjectType::Wasp, Class::Wasp),
        ];
        for (object_type, expected) in projectile_cases {
            let entity = Entity::Projectile(ElementProjectile {
                element: element_data(ElementKind::ObjectProjectile),
                object: object_data(object_type),
                projectile: ProjectileData::default(),
            });
            assert_eq!(
                entity.original_hourglass_class(),
                expected,
                "{object_type:?}"
            );
        }
        for object_type in [ObjectType::Net, ObjectType::BonusNet] {
            let entity = Entity::Net(ElementNet {
                element: element_data(ElementKind::ObjectNet),
                object: object_data(object_type),
                projectile: ProjectileData::default(),
                net: NetData::default(),
            });
            assert_eq!(entity.original_hourglass_class(), Class::Net);
        }
    }

    #[test]
    #[should_panic(expected = "variant/ElementKind invariant failed")]
    fn original_hourglass_mapping_rejects_mismatched_element_kind() {
        Entity::Projectile(ElementProjectile {
            element: element_data(ElementKind::ObjectBonus),
            object: object_data(ObjectType::Arrow),
            projectile: ProjectileData::default(),
        })
        .original_hourglass_class();
    }

    #[test]
    #[should_panic(expected = "no Original concrete-class mapping")]
    fn original_hourglass_mapping_rejects_unproved_variant_object_pair() {
        Entity::Bonus(ElementBonus {
            element: element_data(ElementKind::ObjectBonus),
            object: object_data(ObjectType::Coin),
        })
        .original_hourglass_class();
    }

    #[test]
    fn entity_bonus_object_type_mapping_is_exhaustive_and_original_evidenced() {
        use OriginalBonusConcreteClass as Class;
        let cases = [
            (ObjectType::None, Class::Unsupported),
            (ObjectType::VirtualJumper, Class::Unsupported),
            (ObjectType::VirtualListen, Class::Unsupported),
            (ObjectType::Ale, Class::Ale),
            (ObjectType::Apple, Class::Unsupported),
            (ObjectType::Arrow, Class::Unsupported),
            (ObjectType::Stone, Class::Unsupported),
            (ObjectType::Purse, Class::Unsupported),
            (ObjectType::Coin, Class::Unsupported),
            (ObjectType::Net, Class::Unsupported),
            (ObjectType::Wasp, Class::Unsupported),
            (ObjectType::WaspNest, Class::Unsupported),
            (ObjectType::Scroll, Class::Unsupported),
            (ObjectType::Cape, Class::Cape),
            (ObjectType::BonusAmulet, Class::Bonus),
            (ObjectType::BonusAle, Class::Bonus),
            (ObjectType::BonusApple, Class::Bonus),
            (ObjectType::BonusArrow, Class::Bonus),
            (ObjectType::BonusBlazon, Class::Bonus),
            (ObjectType::BonusLambLeg, Class::Bonus),
            (ObjectType::BonusNet, Class::Bonus),
            (ObjectType::BonusPlants, Class::Bonus),
            (ObjectType::BonusPurse, Class::Bonus),
            (ObjectType::BonusRansom, Class::Bonus),
            (ObjectType::BonusStone, Class::Bonus),
            (ObjectType::BonusWaspNest, Class::Bonus),
            (ObjectType::BonusAmpulla, Class::Bonus),
            (ObjectType::BonusCoronationSpoon, Class::Bonus),
            (ObjectType::BonusRichardsCrown, Class::Bonus),
            (ObjectType::BonusRoyalSeal, Class::Bonus),
            (ObjectType::BonusRoyalSceptre, Class::Bonus),
            (ObjectType::BonusDomesdayBook, Class::Bonus),
            (ObjectType::BonusSwordOfTheState, Class::Bonus),
        ];
        for (object_type, expected) in cases {
            let bonus = ElementBonus {
                object: ObjectData {
                    object_type,
                    ..Default::default()
                },
                element: ElementData::default(),
            };
            assert_eq!(bonus.original_concrete_class(), expected, "{object_type:?}");
        }
    }

    #[test]
    fn element_kind_type_checks() {
        assert!(ElementKind::ActorPc.is_actor());
        assert!(ElementKind::ActorPc.is_human());
        assert!(ElementKind::ActorPc.is_pc());
        assert!(!ElementKind::ActorPc.is_npc());

        assert!(ElementKind::ActorSoldier.is_actor());
        assert!(ElementKind::ActorSoldier.is_human());
        assert!(ElementKind::ActorSoldier.is_npc());
        assert!(ElementKind::ActorSoldier.is_soldier());
        assert!(!ElementKind::ActorSoldier.is_civilian());

        assert!(ElementKind::ActorCivilian.is_civilian());
        assert!(ElementKind::ActorCivilian.is_npc());

        assert!(ElementKind::Fx.is_fx());
        assert!(ElementKind::Target.is_fx());
        assert!(!ElementKind::Fx.is_actor());

        assert!(ElementKind::ObjectBonus.is_object());
        assert!(ElementKind::ObjectBonus.is_bonus());
        assert!(ElementKind::ObjectProjectile.is_projectile());
        assert!(ElementKind::ObjectNet.is_projectile());
    }

    #[test]
    fn posture_checks() {
        assert!(Posture::Dead.is_dead());
        assert!(Posture::DeadBack.is_dead());
        assert!(!Posture::Upright.is_dead());

        assert!(Posture::Lying.is_lying());
        assert!(Posture::Tied.is_lying());
        assert!(!Posture::Upright.is_lying());
    }

    #[test]
    fn target_sprite_visual_anchor_uses_preserved_3d_position() {
        let mut element = ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D::new(120.0, 80.0, 30.0));
        element.set_position_map_preserving_3d(MapPoint::new(10.0, 20.0));
        let target = Entity::Target(ElementTarget {
            element,
            fx: FxData::default(),
            target: TargetData::default(),
        });

        assert_eq!(
            target.sprite_visual_map_position(),
            MapPoint::new(120.0, 50.0)
        );
    }

    #[test]
    fn ground_position_uses_stored_world_xy_not_projected_map_position() {
        let mut element = ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D::new(120.0, 80.0, 30.0));
        element.set_position_map_preserving_3d(MapPoint::new(10.0, 20.0));
        let target = Entity::Target(ElementTarget {
            element,
            fx: FxData::default(),
            target: TargetData::default(),
        });

        assert_eq!(target.ground_position(), GroundPoint::new(120.0, 80.0));
    }

    #[test]
    fn target_hotspots_use_the_exact_cached_sprite_top_left() {
        let mut element = ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        };
        element.sprite.center = crate::coordinates::SpriteAnchor::new(30.0, 140.0);
        element.set_position(WorldPoint3D::new(2821.0, 727.355, 416.355));
        element
            .sprite
            .position_iface
            .set_cached_sprite_position(MapPoint::new(2791.0, 171.0));
        element.set_position_map_preserving_3d(MapPoint::new(2823.0, 312.0));
        let target = Entity::Target(ElementTarget {
            element,
            fx: FxData::default(),
            target: TargetData::default(),
        });

        assert_eq!(
            target.cxx_position_sprite(),
            crate::coordinates::SpriteTopLeft::new(2791.0, 171.0)
        );
    }

    #[test]
    fn actor_sprite_visual_anchor_applies_jump_offset() {
        let pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..ElementData::default()
            },
            actor: ActorData {
                jump_z_offset: 12.0,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let mut pc = pc;
        pc.element_data_mut()
            .set_position_map(MapPoint::new(40.0, 70.0));

        assert_eq!(pc.sprite_visual_map_position(), MapPoint::new(40.0, 58.0));
    }

    /// Corpse-transition guard: a dead corpse can only flip to
    /// `Carried`; every other posture write on a `Dead` / `DeadBack`
    /// sprite is silently dropped.
    #[test]
    fn set_posture_undead_guard() {
        // Alive → any posture is applied normally.
        let mut elem = ElementData {
            posture: Posture::Upright,
            ..Default::default()
        };
        elem.set_posture(Posture::Lying);
        assert_eq!(elem.posture, Posture::Lying);
        elem.set_posture(Posture::Crouched);
        assert_eq!(elem.posture, Posture::Crouched);

        // Dead + non-Carried → silently dropped (no undead!).
        let mut dead = ElementData {
            posture: Posture::Dead,
            ..Default::default()
        };
        dead.set_posture(Posture::Upright);
        assert_eq!(
            dead.posture,
            Posture::Dead,
            "stun on corpse must not revive"
        );
        dead.set_posture(Posture::Lying);
        assert_eq!(dead.posture, Posture::Dead);
        dead.set_posture(Posture::Crouched);
        assert_eq!(dead.posture, Posture::Dead);

        // Dead + Carried → allowed (pickup corpse).
        dead.set_posture(Posture::Carried);
        assert_eq!(dead.posture, Posture::Carried);

        // Same semantics for DeadBack.
        let mut dead_back = ElementData {
            posture: Posture::DeadBack,
            ..Default::default()
        };
        dead_back.set_posture(Posture::Upright);
        assert_eq!(dead_back.posture, Posture::DeadBack);
        dead_back.set_posture(Posture::Carried);
        assert_eq!(dead_back.posture, Posture::Carried);

        // Once flipped to Carried the entity is no longer "dead
        // posture", so normal writes apply again — Carried lifts the
        // lock.
        dead_back.set_posture(Posture::Dead);
        assert_eq!(dead_back.posture, Posture::Dead);
    }

    #[test]
    fn entity_set_posture_updates_authoritative_posture() {
        let mut entity = Entity::Soldier(ActorSoldier {
            element: ElementData {
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        });

        entity.set_posture(Posture::Crouched);

        assert_eq!(entity.element_data().posture, Posture::Crouched);
        assert_eq!(entity.posture(), Posture::Crouched);

        entity.set_posture(Posture::Dead);
        entity.set_posture(Posture::Upright);

        assert_eq!(entity.element_data().posture, Posture::Dead);
        assert_eq!(entity.posture(), Posture::Dead);
    }

    #[test]
    fn posture_allows_transition_to() {
        // Non-dead → any.
        assert!(Posture::Upright.allows_transition_to(Posture::Lying));
        assert!(Posture::Lying.allows_transition_to(Posture::Upright));
        assert!(Posture::Crouched.allows_transition_to(Posture::Dead));
        // Dead → Carried only.
        assert!(Posture::Dead.allows_transition_to(Posture::Carried));
        assert!(Posture::DeadBack.allows_transition_to(Posture::Carried));
        assert!(!Posture::Dead.allows_transition_to(Posture::Upright));
        assert!(!Posture::Dead.allows_transition_to(Posture::Lying));
        assert!(!Posture::DeadBack.allows_transition_to(Posture::Dead));
    }

    #[test]
    fn action_state_groups() {
        assert!(ActionState::Moving.is_moving());
        assert!(ActionState::MovingFast.is_moving());
        assert!(!ActionState::Waiting.is_moving());

        assert!(ActionState::AimingWithBow.is_bow());
        assert!(ActionState::AimingWithBowUp.is_bow());
        assert!(!ActionState::Waiting.is_bow());

        assert!(ActionState::WaitingSword.is_sword());
        assert!(ActionState::ParryingSwordLow.is_sword());
        assert!(!ActionState::Waiting.is_sword());

        assert!(ActionState::HoldingShield.is_shield());
        assert!(ActionState::ParryingShield.is_shield());
        assert!(ActionState::MovingShield.is_shield());
    }

    #[test]
    fn camp_enemy() {
        assert!(Camp::Royalists.is_hostile_to(Camp::Lacklandists));
        assert!(Camp::Lacklandists.is_hostile_to(Camp::Royalists));
        assert!(!Camp::Royalists.is_hostile_to(Camp::Royalists));
        let ten_teams: Vec<_> = (0..10).map(Camp::from_allegiance_id).collect();
        for (left_index, left) in ten_teams.iter().enumerate() {
            for (right_index, right) in ten_teams.iter().enumerate() {
                assert_eq!(left.is_hostile_to(*right), left_index != right_index);
            }
        }
    }

    #[test]
    fn invalid_camp_hostility_remains_nonhostile_after_warning() {
        assert!(!Camp::Error.is_hostile_to(Camp::Royalists));
    }

    #[test]
    fn command_swordstrike() {
        assert!(Command::SwordstrikeThrustA.is_swordstrike());
        assert!(Command::SwordstrikeThrustI.is_swordstrike());
        assert!(!Command::Move.is_swordstrike());
        assert!(!Command::SwordstrikeTired.is_swordstrike());
    }

    #[test]
    fn entity_serde_roundtrip() {
        let pc = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };

        let entity = Entity::Pc(pc);
        let json = serde_json::to_string(&entity).unwrap();
        let back: Entity = serde_json::from_str(&json).unwrap();

        assert!(back.is_pc());
        assert!(back.is_actor());
        assert!(back.is_human());
        assert!(!back.is_npc());
    }

    #[test]
    fn entity_sub_data_accessors() {
        let soldier = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 75,
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        });

        assert!(soldier.element_data().active);
        assert!(soldier.actor_data().is_some());
        assert!(soldier.human_data().is_some());
        assert!(soldier.npc_data().is_some());
        assert!(soldier.object_data().is_none());
    }

    #[test]
    fn entity_is_dead_dispatch() {
        let alive = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 50,
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        });
        assert!(!alive.is_dead());

        let dead = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 0,
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        });
        assert!(dead.is_dead());

        // Non-actors default to dead = true
        let fx = Entity::Fx(ElementFx {
            element: ElementData {
                kind: ElementKind::Fx,
                ..ElementData::default()
            },
            fx: FxData::default(),
        });
        assert!(fx.is_dead());
    }

    #[test]
    fn bonus_is_relic() {
        let mut bonus = ElementBonus {
            element: ElementData {
                kind: ElementKind::ObjectBonus,
                ..ElementData::default()
            },
            object: ObjectData {
                object_type: ObjectType::BonusRichardsCrown,
                ..ObjectData::default()
            },
        };
        assert!(bonus.is_relic());

        bonus.object.object_type = ObjectType::BonusArrow;
        assert!(!bonus.is_relic());
    }

    #[test]
    fn trait_human_on_soldier() {
        let s = ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 50,
                ..NpcData::default()
            },
            soldier: SoldierData {
                cached_max_life_points: 80,
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        };

        assert_eq!(Human::life_points(&s), 50);
        assert!(!s.is_out_of_order());
        assert_eq!(s.camp(), Camp::Lacklandists);
        assert!(s.is_enemy_of(Camp::Royalists));
        assert!(s.is_able_to_fight());
    }

    #[test]
    fn trait_human_on_pc_uses_authored_camp() {
        let pc = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                cached_camp: Camp::Custom(7),
                ..PcData::default()
            },
        };

        assert_eq!(Human::camp(&pc), Camp::Custom(7));
        assert_eq!(Entity::Pc(pc).camp(), Camp::Custom(7));
    }

    #[test]
    fn trait_human_dead_soldier() {
        let s = ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 0,
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        };

        assert!(Element::is_dead(&s));
        assert!(s.is_out_of_order());
        assert!(!s.is_able_to_fight());
    }

    #[test]
    fn trait_pc_immortal() {
        let pc = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                immortal: true,
                ..PcData::default()
            },
        };

        assert!(Element::is_immortal(&pc));
        assert!(!pc.is_robin()); // robin defaults to false
    }

    // ═══════════════════════════════════════════════════════════════
    //  Cross-module integration tests
    // ═══════════════════════════════════════════════════════════════

    #[test]
    fn entity_sprite_reference() {
        use crate::sprite::Sprite;

        let mut entity = Entity::Fx(ElementFx {
            element: ElementData {
                kind: ElementKind::Fx,
                ..ElementData::default()
            },
            fx: FxData::default(),
        });

        // Every entity carries a sprite by default (non-Option).
        assert_eq!(entity.sprite().current_frame, 0);

        // Re-attach a fresh sprite.
        entity.element_data_mut().sprite = Sprite::default();
        assert_eq!(entity.sprite().current_frame, 0);
    }

    #[test]
    fn entity_grid_cell_tracking() {
        let mut data = ElementData {
            kind: ElementKind::ActorPc,
            ..ElementData::default()
        };
        data.set_position_map(MapPoint { x: 200.0, y: 300.0 });

        data.update_grid_cell();
        // 200/64 = 3, 300/64 = 4
        assert_eq!(data.grid_cell, Some((3, 4)));
    }

    #[test]
    fn lying_stars_point_uses_floored_sprite_top_left_plus_hotspot() {
        use crate::sprite::Sprite;
        use crate::sprite_script::SpriteScript;
        use std::sync::Arc;

        let mut sprite = Sprite {
            center: crate::coordinates::SpriteAnchor { x: 30.0, y: 70.0 },
            scripts: Arc::new(vec![SpriteScript {
                hotspot: crate::coordinates::SpriteLocalPoint::new(46.0, 18.0),
                ..SpriteScript::default()
            }]),
            ..Sprite::default()
        };
        sprite.current_row = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            posture: Posture::Lying,
            sprite,
            ..ElementData::default()
        };
        element.set_position_map(MapPoint {
            x: 200.75,
            y: 300.75,
        });

        let entity = Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        });

        let stars = entity.compute_stars_point().unwrap();

        assert_eq!(stars.x, 216.0);
        assert_eq!(stars.y - stars.z, 248.0);
        assert_eq!(stars.z, 5.0);
    }

    // `actor_pathfinder_waypoints` deleted — the waypoint state it
    // covered (ActorData::path_waypoints / path_waypoint_index,
    // set_path, next_waypoint, advance_waypoint, has_path) moved to
    // the active Move element's order queue during the order-queue
    // refactor.  Integration-level coverage now lives in the
    // movement tick tests.

    #[test]
    fn npc_ai_controller_reference() {
        use crate::ai::AiState;
        use crate::ai_enemy::EnemyAi;

        let entity_id = EntityId::Pc(crate::entity_id::PcId(42));
        let mut enemy_ai = EnemyAi::new(7);
        enemy_ai.base.owner_entity_id = Some(entity_id);
        assert_eq!(enemy_ai.base.me, 7);
        assert_eq!(enemy_ai.base.owner_entity_id, Some(entity_id));
        assert_eq!(enemy_ai.base.current_state, AiState::Default);

        let mut soldier = ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        };

        // Attach AI brain
        enemy_ai.base.current_state = AiTopState::Attacking;
        enemy_ai.base.current_substate = AiSubstate::AttackingSwordfight;
        soldier.npc.ai_brain = AiBrain::Enemy(Box::new(enemy_ai));

        // Access via Entity enum
        let entity = Entity::Soldier(soldier);
        assert_eq!(entity.ai_state(), Some(AiTopState::Attacking));

        let ai_ref = entity.ai_controller().unwrap();
        assert_eq!(ai_ref.owner_entity_id, Some(entity_id));

        // Can also access the enemy-specific subclass
        assert!(entity.enemy_ai().is_some());
    }

    #[test]
    fn entity_cross_module_serde_roundtrip() {
        use crate::ai_enemy::EnemyAi;

        // Verify that entities with the new fields still serialize/deserialize.
        let mut enemy_ai = EnemyAi::new(0);
        enemy_ai.base.current_state = AiTopState::Seeking;
        let soldier = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: AiActorData {
                    ai_brain: AiBrain::Enemy(Box::new(enemy_ai)),
                    ..AiActorData::default()
                },
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        });

        let json = serde_json::to_string(&soldier).unwrap();
        let back: Entity = serde_json::from_str(&json).unwrap();

        assert!(back.is_soldier());
        assert_eq!(back.ai_state(), Some(AiTopState::Seeking));
        // Sprite is now serialised (no serde-skip), so it survives round-trip.
        assert_eq!(back.sprite().current_frame, 0);
    }

    #[test]
    fn swordfight_opponents_preserve_record_pairing_during_mutation() {
        let first = EntityId::Pc(crate::entity_id::PcId(11));
        let second = EntityId::Soldier(crate::entity_id::SoldierId(12));
        let first_line = JumpLineIndex::new(21).unwrap();
        let second_line = JumpLineIndex::new(22).unwrap();
        let promoted_line = JumpLineIndex::new(23).unwrap();
        let ignored_line = JumpLineIndex::new(24).unwrap();
        let mut opponents = SwordfightOpponents::from_pairs([
            (first, Some(first_line)),
            (second, Some(second_line)),
        ]);

        assert!(!opponents.add_principal(second, Some(promoted_line)));
        assert_eq!(opponents.ids(), vec![second, first]);
        assert_eq!(opponents.jump_line(0), Some(promoted_line));
        assert_eq!(opponents.jump_line(1), Some(first_line));

        // Original AddOpponent leaves the existing principal's line alone.
        assert!(!opponents.add_principal(second, Some(ignored_line)));
        assert_eq!(opponents.jump_line(0), Some(promoted_line));

        assert!(opponents.remove(first));
        assert_eq!(opponents.ids(), vec![second]);
        assert_eq!(opponents.jump_line(0), Some(promoted_line));
    }

    #[test]
    fn swordfight_opponents_standalone_serde_is_an_ordered_record_sequence() {
        let first = SwordfightOpponent::new(
            EntityId::Pc(crate::entity_id::PcId(25)),
            Some(JumpLineIndex::new(35).unwrap()),
        );
        let second =
            SwordfightOpponent::new(EntityId::Soldier(crate::entity_id::SoldierId(26)), None);
        let entries = vec![first, second];
        let opponents = SwordfightOpponents::from_entries(entries.clone());

        let json = serde_json::to_value(&opponents).unwrap();
        assert!(json.is_array(), "private aggregate storage must not leak");
        assert_eq!(json, serde_json::to_value(&entries).unwrap());
        assert_eq!(
            serde_json::from_value::<SwordfightOpponents>(json).unwrap(),
            opponents
        );

        let binary = bitcode::encode(&opponents);
        let decoded: SwordfightOpponents = bitcode::decode(&binary).unwrap();
        assert_eq!(decoded, opponents);
    }

    #[test]
    fn human_opponents_serde_retains_legacy_fields_and_normalizes_short_lines() {
        let first = EntityId::Pc(crate::entity_id::PcId(31));
        let second = EntityId::Soldier(crate::entity_id::SoldierId(32));
        let line = JumpLineIndex::new(41).unwrap();
        let human = HumanData {
            opponents: SwordfightOpponents::from_pairs([(first, Some(line)), (second, None)]),
            ..HumanData::default()
        };

        let mut value = serde_json::to_value(&human).unwrap();
        assert_eq!(
            value.get("opponents"),
            Some(&serde_json::to_value(vec![first, second]).unwrap())
        );
        assert_eq!(
            value.get("opponent_jump_lines"),
            Some(&serde_json::to_value(vec![Some(line), None]).unwrap())
        );
        assert!(value.get("entries").is_none());

        let json = serde_json::to_string(&human).unwrap();
        let hollow = json.find("\"hollow_man\"").unwrap();
        let opponents = json.find("\"opponents\"").unwrap();
        let jump_lines = json.find("\"opponent_jump_lines\"").unwrap();
        let smalltalk = json.find("\"smalltalk_initiative\"").unwrap();
        assert!(hollow < opponents && opponents < jump_lines && jump_lines < smalltalk);

        let round_trip: HumanData = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(round_trip.opponents, human.opponents);

        let mut predating_jump_lines = value.clone();
        predating_jump_lines
            .as_object_mut()
            .unwrap()
            .remove("opponent_jump_lines");
        let normalized: HumanData = serde_json::from_value(predating_jump_lines).unwrap();
        assert_eq!(normalized.opponents.ids(), vec![first, second]);
        assert_eq!(
            normalized
                .opponents
                .iter_with_jump_lines()
                .collect::<Vec<_>>(),
            vec![(first, None), (second, None)]
        );

        let binary = bitcode::encode(&human);
        let binary_round_trip: HumanData = bitcode::decode(&binary).unwrap();
        assert_eq!(binary_round_trip.opponents, human.opponents);

        value["opponent_jump_lines"] = serde_json::json!([]);
        let normalized: HumanData = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(normalized.opponents.ids(), vec![first, second]);
        assert_eq!(
            normalized
                .opponents
                .iter_with_jump_lines()
                .collect::<Vec<_>>(),
            vec![(first, None), (second, None)]
        );

        value["opponent_jump_lines"] =
            serde_json::to_value(vec![None::<JumpLineIndex>, None, None]).unwrap();
        let error = serde_json::from_value::<HumanData>(value).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("opponent_jump_lines has 3 entries for 2 opponents")
        );
    }

    #[test]
    fn swordfight_opponents_state_hash_matches_legacy_parallel_vectors() {
        use robin_util::state_hash::StateHash;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher as _;

        let opponents = vec![
            EntityId::Pc(crate::entity_id::PcId(51)),
            EntityId::Soldier(crate::entity_id::SoldierId(52)),
        ];
        let jump_lines = vec![Some(JumpLineIndex::new(61).unwrap()), None];
        let aggregate = SwordfightOpponents::from_pairs(
            opponents.iter().copied().zip(jump_lines.iter().copied()),
        );

        let mut legacy_hasher = DefaultHasher::new();
        opponents.state_hash(&mut legacy_hasher);
        jump_lines.state_hash(&mut legacy_hasher);
        let mut aggregate_hasher = DefaultHasher::new();
        aggregate.state_hash(&mut aggregate_hasher);

        assert_eq!(legacy_hasher.finish(), aggregate_hasher.finish());
    }

    #[test]
    fn entity_position_iface_accessor() {
        let pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });

        // PI now always exists (it lives on every sprite).
        assert_eq!(
            pc.position_iface().get_direction(),
            crate::position_interface::Direction::NORTH
        );
    }
}
