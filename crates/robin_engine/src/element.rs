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

use std::fmt;

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
use crate::order::OrderType;
use crate::position_interface::{PositionInterface, SectorHandle};
use crate::profiles::{
    Action, CharacterProfile, CharacterProfileIdx, CivilianProfileIdx, SoldierProfileIdx,
};
use crate::sprite::Sprite;

mod npc;
mod objects;
mod pc;
pub use npc::*;
pub use objects::*;
pub use pc::*;

mod entity;
mod traits;
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
        self.sprite.compute_display_depth();
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
        self.sprite.compute_display_depth();
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

/// Stable reference to an installed canonical actor order.
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
    pub element: crate::sequence::SequenceElementRef,
    pub slot: u64,
}

impl InstalledActorOrder {
    pub fn new(element: crate::sequence::SequenceElementRef, order: &crate::order::Order) -> Self {
        assert_ne!(
            order.storage_slot, 0,
            "installed order must belong to canonical storage"
        );
        Self {
            element,
            slot: order.storage_slot,
        }
    }

    pub fn resolve(self, manager: &crate::sequence::SequenceManager) -> &crate::order::Order {
        manager
            .get_element(self.element.sequence_id, self.element.element_index)
            .and_then(|element| element.orders.resolve(self.slot))
            .expect("installed order storage retired before actor installation boundary")
    }

    pub fn resolve_mut(
        self,
        manager: &mut crate::sequence::SequenceManager,
    ) -> &mut crate::order::Order {
        manager
            .get_element_mut(self.element.sequence_id, self.element.element_index)
            .and_then(|element| element.orders.resolve_mut(self.slot))
            .expect("installed order storage retired before actor installation boundary")
    }
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

    /// Selected instruction, including while its callbacks and translation run.
    pub selected_sequence_element: Option<crate::sequence::SequenceElementRef>,

    /// Actor-update-selected order identity, corresponding to original-game
    /// last-order identity. This is deliberately independent of the sprite's
    /// processed order: FrozenAll still consumes actor initialization once.
    pub last_execute_order_id: Option<std::num::NonZeroU32>,
    /// Stable identity of the installed canonical order, independent of selection.
    pub installed_order: Option<InstalledActorOrder>,
    /// Original-game new-order state for the currently entered execution. Set at
    /// owner selection and cleared after Execute/completion/ActionChange.
    pub execute_order_initialising: bool,

    // Wait
    pub wait_time: u32,

    // Seeking
    pub seek_target: Option<EntityId>,
    pub last_seek_target_position: MapPoint,
    /// Original-game actor seek distance: the unadapted distance
    /// requested by the transient Seek command. Moving-target refreshes
    /// derive their concrete tolerance from this stable base each time.
    pub seek_distance: f32,
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

    /// Destination point for rolling after a death/knockout fall on a slope.
    /// When `combat_anim` finishes and this is set, a Rolling animation is
    /// queued toward this point.
    pub pending_roll: Option<MapPoint>,

    /// World position the shield should face toward during movement.
    /// Set by `dispatch_raise_shield` from the danger point; cleared on
    /// shield lower or combat exit.  Faces the shield toward the threat
    /// rather than the opponent.
    pub shield_face_point: Option<MapPoint>,

    // -- Lift climb state --
    /// If the actor currently owns a ladder-lift reservation, which sector
    /// and which direction. Set at WAIT_FREE_LIFT entry (wall routes do not
    /// contain that action), cleared on the corresponding door-pass exit. Used by
    /// `translate_ladder_wall_fall` to decrement the sector occupancy
    /// counter when a climber gets shoved off.
    pub active_lift: Option<ActiveLiftClimb>,

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
    pub shield_obstacle: Option<Box<crate::sight_obstacle::SightObstacle>>,

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
            selected_sequence_element: None,
            execute_order_initialising: false,

            wait_time: 0,
            seek_target: None,
            last_seek_target_position: MapPoint::default(),
            seek_distance: 0.0,
            post_seek_sequence: None,
            passing_door_directly: false,
            script_class: String::new(),
            pending_roll: None,
            shield_face_point: None,
            active_lift: None,
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
    /// Preserve the shared wait timer, seek distance/last position and independent
    /// order, action, jump, and ability latches for their owning callbacks.
    pub(crate) fn abort_out_of_range_hit_seek(&mut self) {
        self.seek_target = None;
        self.post_seek_sequence = None;
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

/// One entry in the ordered swordfight-opponent list.
///
/// The jump line belongs to this human's side of a table swordfight. Keeping
/// it in the same record as the opponent prevents principal promotion,
/// removal, and insertion from desynchronizing the two values.
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
pub struct SwordfightOpponent {
    opponent: EntityId,
    jump_line: Option<JumpLineIndex>,
}

impl SwordfightOpponent {
    /// Build one paired opponent record.
    ///
    /// The jump line is the line on this human's side of the fight.
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
#[derive(
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

/// Human-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanData {
    pub carrier: Option<EntityId>,
    /// Shared sorting key for nested coordination and target-selection calls.
    /// Every owning operation writes its keys before reading them; a saved
    /// frame never depends on values left by the preceding operation.
    #[state_hash(skip)]
    #[serde(skip)]
    pub sorting_distance: f32,

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

    // Ordered swordfight opponent records.
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

impl Default for HumanData {
    fn default() -> Self {
        Self {
            carrier: None,
            sorting_distance: 0.0,
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
    if (projectile.trajectory_frame_count as i16) <= 0 {
        if projectile.trajectory.is_empty() {
            projectile.flying = false;
            projectile.trajectory_frame_count = u16::MAX;
            projectile.velocity_increment = WorldVec3D::ZERO;
            element.finish_projectile_position_update(WorldVec3D::ZERO);
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
        let time = point.time;
        projectile.trajectory_frame_count = time.wrapping_sub(1);
        let current = element.position();
        let factor = if (time as i16) > 0 {
            1.0 / time as f32
        } else {
            1.0
        };
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
