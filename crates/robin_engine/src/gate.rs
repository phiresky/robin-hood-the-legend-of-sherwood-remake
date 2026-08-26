//! Gate system — doors and jump points that connect sectors in the game world.
//!
//! The serializable state for both regular doors and jump gates is flattened
//! into a single [`Door`] struct, with the type tag carried by enums rather
//! than an inheritance hierarchy.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::ElementKind;
use crate::order::OrderType;
use crate::sector::{LiftType, SectorNumber};

// ---------------------------------------------------------------------------
// DoorIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into the engine's door table.
///
/// Plain `u32` wrapper; no niche optimization because `Option<DoorIndex>`
/// is rare in live fields (most consumers deal with a valid index they
/// already hold).  Existing `door_index: u32` fields will migrate to
/// this type in a follow-up pass; adding the type first keeps the
/// commit reviewable.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DoorIndex(pub u32);

impl From<DoorIndex> for u32 {
    #[inline]
    fn from(i: DoorIndex) -> u32 {
        i.0
    }
}
impl From<DoorIndex> for usize {
    #[inline]
    fn from(i: DoorIndex) -> usize {
        i.0 as usize
    }
}
impl From<u32> for DoorIndex {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v)
    }
}
impl std::fmt::Display for DoorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Top-level gate classification.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum GateType {
    #[default]
    None,
    Door,
    Jump,
}

/// Door sub-type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum DoorType {
    #[default]
    Default,
    Building,
    BuildingTrap,
    Gate,
    LiftHigh,
    LiftLow,
    LiftHighCrenel,
    Trap,
    Reinforcement,
}

// ---------------------------------------------------------------------------
// ActorAuthInfo — actor properties for authorization checks
// ---------------------------------------------------------------------------

/// Properties of an actor relevant for door/gate/lift authorization checks.
///
/// Decouples the authorization logic from the full actor type hierarchy.
/// The caller constructs this from whatever actor representation they have.
#[derive(Debug, Clone, Copy)]
pub struct ActorAuthInfo {
    /// What kind of actor this is (PC, soldier, civilian, animal, etc.).
    pub kind: ElementKind,
    /// For PC actors, the authorization bitmask (e.g. `0x0001` for Cooper).
    /// PC authorization bit — `1 << profile_index`, populated by
    /// [`Element::actor_auth_info`].  Zero for non-PC actors.
    pub pc_auth_bit: u16,
    /// Whether this PC has the lockpick contextual action.
    pub has_lockpick: bool,
    /// Whether this PC has the climb contextual action (for wall lifts).
    pub has_climb: bool,
    /// Whether this PC has the jump contextual action (for jump gates).
    pub has_jump: bool,
    /// Whether this human actor is currently riding a horse.
    pub is_rider: bool,
    /// Actor's current posture (used for lift/door posture restrictions).
    pub posture: crate::element::Posture,
}

// ---------------------------------------------------------------------------
// GateState — physical open/close state machine
// ---------------------------------------------------------------------------

/// Physical state of a gate (drawbridge, portcullis, etc.).
///
/// Explicit state machine for gates that physically open and close, separate
/// from the simpler `active` flag used by other door types.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum GateState {
    /// Gate is fully closed — blocks passage.
    #[default]
    Closed,
    /// Gate is in the process of opening.
    Opening,
    /// Gate is fully open — allows passage.
    Open,
    /// Gate is in the process of closing.
    Closing,
}

impl GateState {
    /// Whether actors can currently pass through.
    pub fn is_passable(self) -> bool {
        matches!(self, Self::Open)
    }

    /// Request the gate to begin opening.
    pub fn request_open(&mut self) {
        match *self {
            Self::Closed | Self::Closing => *self = Self::Opening,
            _ => {}
        }
    }

    /// Request the gate to begin closing.
    pub fn request_close(&mut self) {
        match *self {
            Self::Open | Self::Opening => *self = Self::Closing,
            _ => {}
        }
    }

    /// Mark the current transition as finished.
    pub fn finish_transition(&mut self) {
        match *self {
            Self::Opening => *self = Self::Open,
            Self::Closing => *self = Self::Closed,
            _ => {}
        }
    }

    /// Toggle between open and closed (requests the opposite transition).
    pub fn toggle(&mut self) {
        match *self {
            Self::Closed => *self = Self::Opening,
            Self::Open => *self = Self::Closing,
            // If mid-transition, reverse direction
            Self::Opening => *self = Self::Closing,
            Self::Closing => *self = Self::Opening,
        }
    }
}

/// Squared radius within which a body blocks a door: 400 — i.e. a 20-unit
/// linear radius around each door endpoint.
pub const BODY_DOOR_BLOCK_SQUARE_RADIUS: f32 = 400.0;

/// A link between two gates that share a sector. Used by the gate-graph
/// A* to find multi-gate paths between sectors.
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
pub struct GateLink {
    /// Index of the other door in the global door table.
    pub other_door: DoorIndex,
    /// Sector through which the link passes (the shared sector).
    pub via_sector: crate::sector::SectorNumber,
    /// Exact identity of the shared sector in the runtime sector arena.
    ///
    /// `None` is retained for old/runtime-created door tables which only
    /// carry public sector numbers. Exact and number-only identities are
    /// deliberately never compared with each other.
    #[serde(default)]
    pub via_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    /// In-sector distance between the two gates' nearest entry points.
    pub distance: f32,
}

// ---------------------------------------------------------------------------
// Door struct — the main serializable gate entity
// ---------------------------------------------------------------------------

/// A door connecting two sectors in the game world.
///
/// Carries all serialized door state: lock flags per actor category,
/// unlockability, and special PC authorisations.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Door {
    // -- Gate base fields (serialized by patches) --
    pub gate_type: GateType,
    pub active: bool,

    // -- Door type --
    pub door_type: DoorType,

    // -- Lock state (serialized) --
    pub locked_pc: bool,
    pub locked_npc_villain: bool,
    pub locked_npc_civilian: bool,
    pub unlockable: bool,

    // -- Lock state after patch swap --
    //
    // Both halves are serialized: `swap_rights_patch()` mutates them in
    // place, so saving only the currently-active half would lose future
    // swap behavior.
    pub locked_pc_after_patch: bool,
    pub locked_npc_villain_after_patch: bool,
    pub locked_npc_civilian_after_patch: bool,
    pub unlockable_after_patch: bool,

    // -- Special PC authorisation (serialized) --
    pub special_authorisation_pc: bool,
    /// Bitmask of PC characters authorised in the direct (outside→inside) direction.
    pub authorised_pc_direct: u16,
    /// Bitmask of PC characters authorised in the indirect (inside→outside) direction.
    pub authorised_pc_indirect: u16,

    // -- Geometry loaded from proto --
    /// Entry point on the outside of the door.
    pub point_out: MapPoint,
    /// Entry point on the inside of the door.
    pub point_in: MapPoint,
    /// Mid-point of the door (used for animation offsets).
    pub point_mid: MapPoint,

    /// Layer indices for outside / inside.
    pub layer_out: u16,
    pub layer_in: u16,

    /// Sector numbers for outside / inside. Loaded from proto.
    /// Used by the gate-graph A* (`find_path_gates`) to determine which
    /// motion sector each side of the door belongs to.
    pub sector_out: crate::sector::SectorNumber,
    pub sector_in: crate::sector::SectorNumber,

    /// Exact runtime-arena identities for the two endpoint sectors.
    ///
    /// Original compares `RHSector*` pointers when linking and traversing
    /// gates. Public sector numbers alone are insufficient because distinct
    /// arena objects can expose the same number. `None` is the compatibility
    /// representation for door tables which have not yet been resolved to
    /// the runtime arena.
    #[serde(default)]
    pub sector_out_index: Option<crate::fast_find_grid::SectorIndex>,
    #[serde(default)]
    pub sector_in_index: Option<crate::fast_find_grid::SectorIndex>,

    /// Lift sector which authored this door in its embedded door array.
    ///
    /// This is deliberately stronger than `sector_in`: unrelated standalone
    /// or building doors can lead into the same lift sector, but Original's
    /// `RHSectorLift::mpLowestDoor/mpHighestDoor` candidates are only the
    /// doors embedded in that lift's proto chunk.
    #[serde(default)]
    pub owning_lift_sector: Option<crate::sector::SectorNumber>,

    /// Linked gates: indices of doors that share a sector with this one.
    /// Each link's distance is the in-sector distance between the two
    /// gates' nearest entry points. Built once at level load.
    pub gate_links: Vec<GateLink>,

    /// Polygon defining the clickable mouse-sector area for this door.
    /// Loaded from the proto stream's `door_sector` polygon and used by
    /// the engine's sector hit-test as a mouse-pickable region.
    ///
    /// Empty for doors loaded from save files where the polygon wasn't
    /// available (the proto stream is the authoritative source).
    pub click_polygon: Vec<(f32, f32)>,

    /// Axis-aligned bounding box of `click_polygon`. Pre-computed for
    /// fast rejection in hit-tests.
    pub click_bbox: crate::coordinates::MapBBox,

    /// Pathfinding penalty for crossing this door.
    pub penalty: f32,

    /// Index into the canonical mission patch table. Mirrors the C++
    /// `pDoor->mpPatch` link: populated only for `door_triggered`
    /// patches, where opening/passing this door fires the patch
    /// (consumed by `apply_door_patch` and `gate_state.finish_transition`).
    /// `triggers_door`-style links are stored on the *patch* side as
    /// `Patch::door_indices` instead.
    pub patch_index: Option<crate::patch::PatchIndex>,

    /// Physical open/close state for gate-type doors (drawbridge, portcullis).
    /// Tracks opening/closing animation progress.
    /// Building doors use the patch system for visual state instead.
    pub gate_state: GateState,

    /// For `GateType::Jump` gates only: index of the "out-side"
    /// jump line (the line on the `sector_out` side of the jump).
    pub jump_line_out: Option<u32>,

    /// For `GateType::Jump` gates only: index of the "in-side"
    /// jump line (the line on the `sector_in` side of the jump).
    pub jump_line_in: Option<u32>,

    /// For `GateType::Jump` gates only: cached `helper_needed` flag
    /// of `jump_line_in` (the destination line when `direct = true`).
    ///
    /// Populated by `load_jump_lines_from_proto` from the paired jump
    /// zone's `helper_needed` bit (ultimately `JumpLine.helper_needed`
    /// at `jump_line.rs:L112`).  Cached on the door so
    /// [`Door::is_actor_authorized`] can decide the destination-line
    /// branch without needing a back-reference into
    /// `fast_grid.level.jump_lines` — `Door` has no knowledge of the
    /// grid at authorization time.
    pub jump_line_in_helper_needed: bool,

    /// For `GateType::Jump` gates only: cached `helper_needed` flag
    /// of `jump_line_out` (the destination line when `direct = false`).
    ///
    /// See [`jump_line_in_helper_needed`](Self::jump_line_in_helper_needed).
    pub jump_line_out_helper_needed: bool,

    /// Per-door entry-step animation hints for `get_action_1`. Not
    /// serialized — derived from the door type at load time by
    /// [`Door::default_actions_for_type`].
    pub action_direct_1: OrderType,
    pub action_indirect_1: OrderType,
    /// Per-door exit-step animation hints for `get_action_2`. See
    /// [`action_direct_1`](Self::action_direct_1) for the load-time
    /// derivation and lifetime.
    pub action_direct_2: OrderType,
    pub action_indirect_2: OrderType,
}

impl Default for Door {
    fn default() -> Self {
        Self {
            gate_type: GateType::Door,
            active: true,
            door_type: DoorType::Default,
            locked_pc: false,
            locked_npc_villain: false,
            locked_npc_civilian: false,
            unlockable: false,
            locked_pc_after_patch: false,
            locked_npc_villain_after_patch: false,
            locked_npc_civilian_after_patch: false,
            unlockable_after_patch: false,
            special_authorisation_pc: false,
            authorised_pc_direct: 0,
            authorised_pc_indirect: 0,
            point_out: MapPoint::ZERO,
            point_in: MapPoint::ZERO,
            point_mid: MapPoint::ZERO,
            layer_out: 0,
            layer_in: 0,
            sector_out: crate::sector::SectorNumber::new(0),
            sector_in: crate::sector::SectorNumber::new(0),
            sector_out_index: None,
            sector_in_index: None,
            owning_lift_sector: None,
            gate_links: Vec::new(),
            click_polygon: Vec::new(),
            click_bbox: crate::coordinates::MapBBox::new(),
            penalty: 0.0,
            patch_index: None,
            gate_state: GateState::default(),
            jump_line_out: None,
            jump_line_in: None,
            jump_line_in_helper_needed: false,
            jump_line_out_helper_needed: false,
            action_direct_1: OrderType::WalkingUpright,
            action_indirect_1: OrderType::WalkingUpright,
            action_direct_2: OrderType::WalkingUpright,
            action_indirect_2: OrderType::WalkingUpright,
        }
    }
}

impl Door {
    /// Test whether a map-space point lies inside this door's clickable
    /// polygon. Used by the engine sector hit-test to detect clicks on a
    /// door / drawbridge surface.
    ///
    /// Returns `false` if the door is inactive, has no polygon loaded,
    /// or the point falls outside the polygon's bbox.
    pub fn click_polygon_contains(&self, x: f32, y: f32) -> bool {
        if !self.active || self.click_polygon.len() < 3 {
            return false;
        }
        // Fast bbox reject.
        let pt = crate::coordinates::MapPoint::new(x, y);
        if !self.click_bbox.contains_point(pt) {
            return false;
        }
        // Original `SBGeoPolygon2DArray::IsInside` counts polygon boundaries
        // as inside. This matters for recorded group moves: H02_Not_EC can
        // select door sector 296 at x=1669, exactly on its right edge.
        // Check the closed edge before applying the half-open ray toggle.
        let mut inside = false;
        let n = self.click_polygon.len();
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = self.click_polygon[i];
            let (xj, yj) = self.click_polygon[j];
            let edge_x = xj - xi;
            let edge_y = yj - yi;
            let rel_x = x - xi;
            let rel_y = y - yi;
            if edge_x * rel_y == edge_y * rel_x
                && x >= xi.min(xj)
                && x <= xi.max(xj)
                && y >= yi.min(yj)
                && y <= yi.max(yj)
            {
                return true;
            }
            if (yi > y) != (yj > y) {
                let x_intersect = (xj - xi) * (y - yi) / (yj - yi) + xi;
                if x < x_intersect {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Recompute `click_bbox` from `click_polygon`.
    pub fn rebuild_click_bbox(&mut self) {
        let mut bbox = crate::coordinates::MapBBox::new();
        for &(x, y) in &self.click_polygon {
            bbox.expand_point(crate::coordinates::MapPoint::new(x, y));
        }
        self.click_bbox = bbox;
    }
}

// ---------------------------------------------------------------------------
// Lock / unlock API
// ---------------------------------------------------------------------------

impl Door {
    // -- PC locks --

    pub fn is_locked_pc(&self) -> bool {
        self.locked_pc
    }

    pub fn set_locked_pc(&mut self, locked: bool) {
        self.locked_pc = locked;
    }

    pub fn lock_pc(&mut self) {
        self.locked_pc = true;
    }

    pub fn unlock_pc(&mut self) {
        self.locked_pc = false;
    }

    // -- NPC villain locks --

    pub fn is_locked_npc_villain(&self) -> bool {
        self.locked_npc_villain
    }

    pub fn set_locked_npc_villain(&mut self, locked: bool) {
        self.locked_npc_villain = locked;
    }

    pub fn lock_npc_villain(&mut self) {
        self.locked_npc_villain = true;
    }

    pub fn unlock_npc_villain(&mut self) {
        self.locked_npc_villain = false;
    }

    // -- NPC civilian locks --

    pub fn is_locked_npc_civilian(&self) -> bool {
        self.locked_npc_civilian
    }

    pub fn set_locked_npc_civilian(&mut self, locked: bool) {
        self.locked_npc_civilian = locked;
    }

    pub fn lock_npc_civilian(&mut self) {
        self.locked_npc_civilian = true;
    }

    pub fn unlock_npc_civilian(&mut self) {
        self.locked_npc_civilian = false;
    }

    // -- Unlockable (can a PC with lockpick open this door?) --

    pub fn is_unlockable(&self) -> bool {
        self.unlockable
    }

    pub fn set_unlockable(&mut self, unlockable: bool) {
        self.unlockable = unlockable;
    }

    // -- Active state --

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    // -- Type queries --

    pub fn is_door(&self) -> bool {
        self.gate_type == GateType::Door
    }

    pub fn is_jump(&self) -> bool {
        self.gate_type == GateType::Jump
    }

    // -- Special PC authorisation --

    /// Returns `true` if the door is locked for PCs but has a special
    /// authorisation list that might allow specific characters through.
    pub fn has_special_authorisation(&self) -> bool {
        self.locked_pc && self.special_authorisation_pc
    }

    /// Check whether a specific PC (identified by a bit index) is authorised
    /// in the given direction.
    pub fn has_special_authorisation_for(&self, pc_bit: u16, direct: bool) -> bool {
        if !self.has_special_authorisation() {
            return false;
        }
        let mask = if direct {
            self.authorised_pc_direct
        } else {
            self.authorised_pc_indirect
        };
        mask & pc_bit != 0
    }

    /// Grant special authorisation to a PC (bit) in the given direction.
    pub fn grant_special_authorisation(&mut self, pc_bit: u16, direct: bool) {
        if direct {
            self.authorised_pc_direct |= pc_bit;
        } else {
            self.authorised_pc_indirect |= pc_bit;
        }
        self.special_authorisation_pc = true;
    }

    // -- Patch swap --

    /// Swap the current lock/unlockable state with the "after patch" state.
    ///
    /// Used when a game patch is applied or reverted to toggle door access
    /// rights between two configurations.
    pub fn swap_rights_patch(&mut self) {
        std::mem::swap(&mut self.locked_pc, &mut self.locked_pc_after_patch);
        std::mem::swap(
            &mut self.locked_npc_villain,
            &mut self.locked_npc_villain_after_patch,
        );
        std::mem::swap(
            &mut self.locked_npc_civilian,
            &mut self.locked_npc_civilian_after_patch,
        );
        std::mem::swap(&mut self.unlockable, &mut self.unlockable_after_patch);
    }

    // -- Geometry helpers --

    pub fn point_out(&self) -> MapPoint {
        self.point_out
    }

    pub fn point_in(&self) -> MapPoint {
        self.point_in
    }

    pub fn point_mid(&self) -> MapPoint {
        self.point_mid
    }

    pub fn set_point_out(&mut self, x: f32, y: f32) {
        self.point_out = MapPoint::new(x, y);
    }

    pub fn set_point_in(&mut self, x: f32, y: f32) {
        self.point_in = MapPoint::new(x, y);
    }

    pub fn set_point_mid(&mut self, x: f32, y: f32) {
        self.point_mid = MapPoint::new(x, y);
    }

    /// Shift `point_in` so it lies a fixed distance from `point_mid`
    /// along the normalized (`point_in` − `point_mid`) direction.
    ///
    /// * `DoorType::BuildingTrap`: unconditional 60-unit offset.
    /// * `DoorType::LiftHigh` + `lift_wall`: 60-unit offset (only when
    ///   the in-sector's lift type is `Wall` — the caller must supply
    ///   that bit).
    /// * `DoorType::LiftHighCrenel` + `lift_wall`: 65-unit offset.
    /// * All other door types: no-op.
    ///
    /// Called from the MissionLevelBuilder door and lift stages.
    /// after `point_in`/`point_mid` are read from the proto stream and
    /// before `penalty` is computed — the new `point_in` feeds into
    /// `penalty = |point_in - point_out|`, A* gate-graph distances, and
    /// body-blocking geometry.
    ///
    /// If `point_in == point_mid` (degenerate zero-length direction)
    /// the function leaves `point_in` unchanged.
    pub fn adapt_points(&mut self, lift_wall: bool) {
        let offset = match self.door_type {
            DoorType::BuildingTrap => 60.0,
            DoorType::LiftHigh if lift_wall => 60.0,
            DoorType::LiftHighCrenel if lift_wall => 65.0,
            _ => return,
        };
        let dx = self.point_in.x - self.point_mid.x;
        let dy = self.point_in.y - self.point_mid.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON {
            return;
        }
        let inv = offset / len;
        self.point_in = MapPoint::new(self.point_mid.x + dx * inv, self.point_mid.y + dy * inv);
    }

    /// Compute the A* gate-graph traversal penalty for this door.
    ///
    /// `penalty = |point_in - point_out| + (PENALTY_BUILDING for
    /// Building/BuildingTrap, else PENALTY_DEFAULT)`.
    ///
    /// Called from the door populators after [`adapt_points`] has
    /// adjusted `point_in`, because `|point_in - point_out|` must
    /// see the final (shifted) `point_in` value.
    pub fn compute_door_penalty(&mut self) {
        let dx = self.point_in.x - self.point_out.x;
        let dy = self.point_in.y - self.point_out.y;
        let base = (dx * dx + dy * dy).sqrt();
        let extra = match self.door_type {
            DoorType::Building | DoorType::BuildingTrap => PENALTY_BUILDING,
            _ => PENALTY_DEFAULT,
        };
        self.penalty = base + extra;
    }
}

// ---------------------------------------------------------------------------
// Per-door action picker
// ---------------------------------------------------------------------------

impl Door {
    /// Default per-door entry/exit animations for a given door type.
    /// Returns `(direct_1, direct_2, indirect_1, indirect_2)`. Lift and
    /// reinforcement door types fall back to `WalkingUpright`; only
    /// trap doors install a non-trivial entry/exit animation.
    pub fn default_actions_for_type(
        door_type: DoorType,
    ) -> (OrderType, OrderType, OrderType, OrderType) {
        use OrderType as A;
        match door_type {
            // (walking-upright, waiting-crouched → ladder-down,
            //  ladder-up → waiting-crouched, walking-upright)
            DoorType::Trap | DoorType::BuildingTrap => (
                A::WalkingUpright,
                A::TransitionWaitingCrouchedClimbingLadderDown,
                A::TransitionClimbingLadderUpWaitingCrouched,
                A::WalkingUpright,
            ),
            // Default, Building, Gate: all walking-upright.
            // Lift / Reinforcement door types are not given a meaningful
            // animation here; they default to walking-upright too.
            _ => (
                A::WalkingUpright,
                A::WalkingUpright,
                A::WalkingUpright,
                A::WalkingUpright,
            ),
        }
    }

    /// Pick the entry-step animation for a door traversal.
    ///
    ///   * `Default` / `Gate` doors keep an explicit `WalkingCrouched`
    ///     request so a crouched actor never gets promoted to upright.
    ///   * If the per-door hint is a walking variant and the request is
    ///     `RunningUpright`, the running animation is preserved (the door
    ///     doesn't force a slowdown).
    ///   * Otherwise the per-door hint wins (forces ladder transition,
    ///     stairs walk, etc.).
    pub fn get_action_1(&self, direct: bool, action: OrderType) -> OrderType {
        use OrderType as A;
        if matches!(self.door_type, DoorType::Default | DoorType::Gate)
            && action == A::WalkingCrouched
        {
            return A::WalkingCrouched;
        }
        let hint = if direct {
            self.action_direct_1
        } else {
            self.action_indirect_1
        };
        if action == A::RunningUpright && (hint == A::WalkingUpright || hint == A::WalkingStairs) {
            A::RunningUpright
        } else {
            hint
        }
    }

    /// Pick the exit-step animation for a door traversal.  Symmetric to
    /// [`get_action_1`](Self::get_action_1).
    pub fn get_action_2(&self, direct: bool, action: OrderType) -> OrderType {
        use OrderType as A;
        if matches!(self.door_type, DoorType::Default | DoorType::Gate)
            && action == A::WalkingCrouched
        {
            return A::WalkingCrouched;
        }
        let hint = if direct {
            self.action_direct_2
        } else {
            self.action_indirect_2
        };
        if action == A::RunningUpright && (hint == A::WalkingUpright || hint == A::WalkingStairs) {
            A::RunningUpright
        } else {
            hint
        }
    }
}

// ---------------------------------------------------------------------------
// Door authorization
// ---------------------------------------------------------------------------

impl Door {
    /// Check whether an actor is authorized to pass through this door.
    ///
    /// For `Building` / `BuildingTrap` doors entering in the `direct` direction,
    /// the caller must supply `building_has_capacity` from
    /// `BuildingData::is_authorized()`. For all other door types, pass `true`.
    ///
    /// For `LiftHigh` / `LiftLow` / `LiftHighCrenel` doors, this method checks
    /// rider restrictions but does NOT check lift-type restrictions (wall / ladder
    /// / stairs). The caller must additionally check
    /// `LiftType::is_actor_authorized()`.
    pub fn is_actor_authorized(
        &self,
        direct: bool,
        actor: &ActorAuthInfo,
        building_has_capacity: bool,
        allow_leave_map: bool,
    ) -> bool {
        // Jump gates have their own authorization path.  This implements the
        // strictest variant (test posture, do not pass-through on missing
        // posture) — A* pathfinding and authorization pre-checks must never
        // route a PC through an un-helped jump.  The looser variant used by
        // `engine/jump.rs::is_jumpable` is inlined there instead of going
        // through this method.
        if self.gate_type == GateType::Jump {
            if !(actor.kind.is_pc() && actor.has_jump) {
                return false;
            }
            // `helper_needed` lives on the *destination* line:
            //   * direct  ⇒ actor jumps out-side → in-side; destination is
            //     `jump_line_in`.
            //   * !direct ⇒ actor jumps in-side → out-side; destination is
            //     `jump_line_out`.
            let helper_needed = if direct {
                self.jump_line_in_helper_needed
            } else {
                self.jump_line_out_helper_needed
            };
            if !helper_needed {
                return true;
            }
            return actor.posture == crate::element::Posture::OnShoulders;
        }

        // Reinforcement doors never allow passage unless explicitly permitted
        if !allow_leave_map && self.door_type == DoorType::Reinforcement {
            return false;
        }

        // Special PC authorisation overrides normal lock checks
        if self.locked_pc && self.special_authorisation_pc && actor.kind.is_pc() {
            return self.has_special_authorisation_for(actor.pc_auth_bit, direct);
        }

        if !self.active {
            return false;
        }

        match self.door_type {
            DoorType::Building => self.check_building_door(direct, actor, building_has_capacity),
            DoorType::BuildingTrap => {
                self.check_building_trap_door(direct, actor, building_has_capacity)
            }
            DoorType::Default | DoorType::Gate | DoorType::Trap | DoorType::Reinforcement => {
                self.check_standard_door(actor)
            }
            DoorType::LiftHigh | DoorType::LiftLow | DoorType::LiftHighCrenel => {
                // Riders can never use lifts
                if actor.kind.is_human() && actor.is_rider {
                    return false;
                }
                // Actual lift-type check (wall/ladder/stairs) must be done by
                // the caller via LiftType::is_actor_authorized().
                true
            }
        }
    }

    /// Building door authorization.
    fn check_building_door(
        &self,
        direct: bool,
        actor: &ActorAuthInfo,
        building_has_capacity: bool,
    ) -> bool {
        if actor.kind.is_pc() {
            return self.check_pc_lock(actor);
        }
        if actor.kind.is_npc() {
            // NPCs entering require building to have capacity
            if direct && !building_has_capacity {
                return false;
            }
            if actor.kind.is_civilian() {
                return !self.locked_npc_civilian;
            }
            if actor.kind.is_soldier() {
                // Riders cannot enter buildings
                if actor.is_rider {
                    return false;
                }
                return !self.locked_npc_villain;
            }
            // NPC that is neither civilian nor soldier — should not happen
            panic!(
                "unexpected NPC kind in building door auth: {:?}",
                actor.kind
            );
        }
        false
    }

    /// Building-trap door authorization.
    ///
    /// Same as building doors except civilians are *never* authorized.
    fn check_building_trap_door(
        &self,
        direct: bool,
        actor: &ActorAuthInfo,
        building_has_capacity: bool,
    ) -> bool {
        if actor.kind.is_pc() {
            return self.check_pc_lock(actor);
        }
        if actor.kind.is_npc() {
            if direct && !building_has_capacity {
                return false;
            }
            // Civilians never pass through trap doors
            if actor.kind.is_civilian() {
                return false;
            }
            if actor.kind.is_soldier() {
                if actor.is_rider {
                    return false;
                }
                return !self.locked_npc_villain;
            }
            panic!(
                "unexpected NPC kind in building-trap door auth: {:?}",
                actor.kind
            );
        }
        false
    }

    /// Standard door authorization (Default, Gate, Trap, Reinforcement).
    fn check_standard_door(&self, actor: &ActorAuthInfo) -> bool {
        if actor.kind.is_pc() {
            return self.check_pc_lock(actor);
        }
        if actor.kind.is_npc() {
            if actor.kind.is_civilian() {
                return !self.locked_npc_civilian;
            }
            if actor.kind.is_soldier() {
                return !self.locked_npc_villain;
            }
            panic!(
                "unexpected NPC kind in standard door auth: {:?}",
                actor.kind
            );
        }
        false
    }

    /// Common PC lock check: locked → lockpick ? unlockable : false, unlocked → true.
    fn check_pc_lock(&self, actor: &ActorAuthInfo) -> bool {
        if self.locked_pc {
            if actor.has_lockpick {
                self.unlockable
            } else {
                false
            }
        } else {
            true
        }
    }

    /// Check whether a body at the given position would block this door.
    ///
    /// `pos_sector_in` / `pos_sector_out`: the sector index the body is in.
    /// `sq_dist_to_in` / `sq_dist_to_out`: squared distance from body to
    /// the door's in/out points respectively.
    pub fn body_would_block(
        &self,
        body_sector: crate::sector::SectorNumber,
        sector_in: crate::sector::SectorNumber,
        sector_out: crate::sector::SectorNumber,
        sq_dist_to_in: f32,
        sq_dist_to_out: f32,
    ) -> bool {
        if body_sector == sector_in && sq_dist_to_in <= BODY_DOOR_BLOCK_SQUARE_RADIUS {
            return true;
        }
        if body_sector == sector_out && sq_dist_to_out <= BODY_DOOR_BLOCK_SQUARE_RADIUS {
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Gate-graph A* pathfinding
// ---------------------------------------------------------------------------

/// One step in a gate-path: which door to traverse and in which direction.
///
/// `direct = true` means going from `point_out` (sector_out side) to
/// `point_in` (sector_in side). `direct = false` means the reverse.
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
pub struct GatePathStep {
    pub door_index: DoorIndex,
    pub direct: bool,
}

/// Authoritative gate-search outcome captured by an Original parity trace.
///
/// Live gameplay never sets this: it exists so replay can preserve both a
/// successful gate chain and an observed `FindPathGates` failure instead of
/// silently running a second search against reconstructed topology.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RecordedGatePath {
    pub source_sector: crate::sector::SectorNumber,
    pub source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    pub source_layer: u16,
    pub outcome: RecordedGateOutcome,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum RecordedGateOutcome {
    Success(Vec<GatePathStep>),
    Failure,
}

fn is_actor_authorized_for_gate<F>(
    door: &Door,
    direct: bool,
    actor: &ActorAuthInfo,
    building_has_capacity: bool,
    allow_leave_map: bool,
    sector_lift_type: &F,
) -> bool
where
    F: Fn(SectorNumber) -> Option<LiftType>,
{
    if !door.is_actor_authorized(direct, actor, building_has_capacity, allow_leave_map) {
        return false;
    }
    if !matches!(
        door.door_type,
        DoorType::LiftHigh | DoorType::LiftLow | DoorType::LiftHighCrenel
    ) {
        return true;
    }

    let lift_type = sector_lift_type(door.sector_in).unwrap_or_else(|| {
        panic!(
            "lift door {:?} has non-lift sector_in {}",
            door.door_type, door.sector_in
        )
    });
    lift_type.is_actor_authorized(actor)
}

#[inline]
fn building_has_capacity(
    door: &Door,
    direct: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
) -> bool {
    !direct
        || !matches!(door.door_type, DoorType::Building | DoorType::BuildingTrap)
        || building_is_authorized(door.sector_in)
}

/// Rebuild the `gate_links` on every door so that gates sharing a
/// motion sector are linked.  Each motion-area sector collects every
/// gate that touches it; the cross product of those sets becomes the
/// `gate_links` between each pair.
///
/// Called after the door table is finalised (regular doors plus any
/// jump gates from `load_jump_lines_from_proto`) so both kinds are
/// routed through by `find_path_gates`.
struct DoorEndpoints {
    sector_out: crate::sector::SectorNumber,
    sector_in: crate::sector::SectorNumber,
    sector_out_index: Option<crate::fast_find_grid::SectorIndex>,
    sector_in_index: Option<crate::fast_find_grid::SectorIndex>,
    point_out: MapPoint,
    point_in: MapPoint,
}

/// Sector identity used by the gate graph.
///
/// Exact arena identities compare only with exact arena identities. Numeric
/// identities compare only with numeric identities. This prevents a partly
/// migrated graph from silently joining an exact endpoint to an unrelated
/// number-only endpoint which happens to expose the same public number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GateSectorKey {
    Exact(crate::fast_find_grid::SectorIndex),
    Numeric(crate::sector::SectorNumber),
}

impl GateSectorKey {
    #[inline]
    fn new(
        number: crate::sector::SectorNumber,
        index: Option<crate::fast_find_grid::SectorIndex>,
    ) -> Self {
        index.map_or(Self::Numeric(number), Self::Exact)
    }
}

pub fn build_gate_links(doors: &mut [Door]) {
    /// `(door_index, endpoint_xy)` — what lives on each shared sector.
    type SectorEntry = (u32, MapPoint);

    let mut by_sector: std::collections::HashMap<GateSectorKey, Vec<SectorEntry>> =
        std::collections::HashMap::new();
    for (idx, door) in doors.iter().enumerate() {
        let idx_u32 = idx as u32;
        by_sector
            .entry(GateSectorKey::new(door.sector_out, door.sector_out_index))
            .or_default()
            .push((idx_u32, door.point_out));
        by_sector
            .entry(GateSectorKey::new(door.sector_in, door.sector_in_index))
            .or_default()
            .push((idx_u32, door.point_in));
    }

    for d in doors.iter_mut() {
        d.gate_links.clear();
    }

    // Snapshot the per-door endpoints up front so the inner loop can
    // borrow `doors[idx].gate_links` mutably without aliasing.
    let endpoints: Vec<DoorEndpoints> = doors
        .iter()
        .map(|d| DoorEndpoints {
            sector_out: d.sector_out,
            sector_in: d.sector_in,
            sector_out_index: d.sector_out_index,
            sector_in_index: d.sector_in_index,
            point_out: d.point_out,
            point_in: d.point_in,
        })
        .collect();

    for (door_idx, ep) in endpoints.into_iter().enumerate() {
        for (my_sector, my_sector_index, my_point) in [
            (ep.sector_out, ep.sector_out_index, ep.point_out),
            (ep.sector_in, ep.sector_in_index, ep.point_in),
        ] {
            let my_key = GateSectorKey::new(my_sector, my_sector_index);
            if let Some(neighbors) = by_sector.get(&my_key) {
                for &(other_idx, other_point) in neighbors {
                    if other_idx as usize == door_idx {
                        continue;
                    }
                    let dx = other_point.x - my_point.x;
                    let dy = other_point.y - my_point.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    doors[door_idx].gate_links.push(GateLink {
                        other_door: DoorIndex(other_idx),
                        via_sector: my_sector,
                        via_sector_index: my_sector_index,
                        distance: dist,
                    });
                }
            }
        }
    }
}

/// A* search state for one gate during gate-graph pathfinding.
///
/// Stored in a side-table (not on the Door struct) so multiple concurrent
/// searches don't clobber each other.
#[derive(Debug, Clone, Copy)]
struct GateSearchState {
    visited: bool,
    direct: bool,
    distance_from_source: f32,
    score: f32,
    /// The gate we came from on the best path.
    prev_gate: Option<DoorIndex>,
}

impl Default for GateSearchState {
    fn default() -> Self {
        Self {
            visited: false,
            direct: false,
            distance_from_source: f32::INFINITY,
            score: f32::INFINITY,
            prev_gate: None,
        }
    }
}

/// Port of `RHFastFindGrid::AddToListOpenGates`.
///
/// Gate scores live in the shared state table and may decrease while that
/// gate is already present in `open`, so the list can become temporarily
/// unsorted. Original deliberately performs a linear scan over those live
/// scores. A binary search is invalid here and can leave an improved gate
/// behind a more expensive goal node.
fn add_to_open_gates_original(
    open: &mut Vec<DoorIndex>,
    state: &[GateSearchState],
    gate: DoorIndex,
) {
    let score = state[usize::from(gate)].score;
    if open.is_empty() {
        open.push(gate);
        return;
    }

    let mut index = 0;
    let mut open_gate = open[0];
    while score > state[usize::from(open_gate)].score {
        index += 1;
        if index < open.len() {
            open_gate = open[index];
            if open_gate == gate {
                return;
            }
        } else {
            break;
        }
    }
    open.insert(index, gate);
}

#[inline]
fn dist(a: MapPoint, b: MapPoint) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// A* on the gate connectivity graph from `source_sector` to `goal_sector`.
///
/// Returns an ordered list of `GatePathStep` from source to goal, or
/// `None` if no path exists.
///
/// `auth` is used to check if the actor can traverse each gate (locked
/// doors, lift restrictions, etc.). When `Some`, doors that fail the
/// authorization check are skipped — the path will route around them.
/// When `None`, all active gates are accepted.
///
/// `allow_leave_map` is set on map-leaving move orders: when true,
/// reinforcement doors are not hard-blocked by `is_actor_authorized`.
///
/// `building_is_authorized` mirrors `RHSectorBuilding::IsAuthorized`.
/// Original checks it whenever an NPC route enters a building door in the
/// direct direction, both while seeding the search and while expanding it.
#[allow(clippy::too_many_arguments)]
pub fn find_path_gates(
    doors: &[Door],
    source: (f32, f32),
    source_sector: u16,
    goal: (f32, f32),
    goal_sector: u16,
    auth: Option<&ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<Vec<GatePathStep>> {
    find_path_gates_with_sector_indices(
        doors,
        source,
        source_sector,
        None,
        goal,
        goal_sector,
        None,
        auth,
        allow_leave_map,
        building_is_authorized,
        sector_lift_type,
    )
}

/// Identity-aware gate A*.
///
/// Supplying an arena identity requires matching exact identities on door
/// endpoints. Omitting it requires number-only endpoints. The two modes are
/// intentionally not mixed; callers must resolve both sides before entering
/// the exact graph.
#[allow(clippy::too_many_arguments)]
pub fn find_path_gates_with_sector_indices(
    doors: &[Door],
    source: (f32, f32),
    source_sector: u16,
    source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal: (f32, f32),
    goal_sector: u16,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    auth: Option<&ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<Vec<GatePathStep>> {
    let source = MapPoint::new(source.0, source.1);
    let goal = MapPoint::new(goal.0, goal.1);
    let source_key =
        GateSectorKey::new(SectorNumber::new(source_sector as i16), source_sector_index);
    let goal_key = GateSectorKey::new(SectorNumber::new(goal_sector as i16), goal_sector_index);
    if source_key == goal_key {
        return Some(Vec::new());
    }
    if doors.is_empty() {
        return None;
    }

    let n = doors.len();
    let mut state: Vec<GateSearchState> = vec![GateSearchState::default(); n];
    // Open list: indices of gates to expand. Original inserts by a linear scan
    // over live scores; decreasing an already-queued gate can temporarily make
    // older entries unsorted.
    let mut open: Vec<DoorIndex> = Vec::new();

    // ── Seed: gates touching the source sector ──
    for (idx, door) in doors.iter().enumerate() {
        if !door.active {
            continue;
        }
        // Authorization check: skip doors the actor cannot pass. Original
        // tests building capacity during path construction, before an actor
        // can be dispatched toward the door.
        if let Some(a) = auth {
            let direct_candidate =
                GateSectorKey::new(door.sector_out, door.sector_out_index) == source_key;
            if !is_actor_authorized_for_gate(
                door,
                direct_candidate,
                a,
                building_has_capacity(door, direct_candidate, building_is_authorized),
                allow_leave_map,
                sector_lift_type,
            ) {
                continue;
            }
        }

        let (direct, from_pt, to_pt) =
            if GateSectorKey::new(door.sector_out, door.sector_out_index) == source_key {
                // Direct: enter from outside, exit through point_in (into sector_in)
                (true, door.point_out, door.point_in)
            } else if GateSectorKey::new(door.sector_in, door.sector_in_index) == source_key {
                // Indirect: enter from inside, exit through point_out (into sector_out)
                (false, door.point_in, door.point_out)
            } else {
                continue;
            };

        let d_from_src = dist(source, from_pt);
        let d_to_goal = dist(to_pt, goal);
        let score = d_from_src + d_to_goal + door.penalty;

        state[idx] = GateSearchState {
            visited: true,
            direct,
            distance_from_source: d_from_src,
            score,
            prev_gate: None,
        };
        add_to_open_gates_original(&mut open, &state, DoorIndex(idx as u32));
    }

    if open.is_empty() {
        return None;
    }
    // ── A* main loop ──
    while let Some(current_idx) = open.first().copied() {
        open.remove(0);

        let current = &doors[usize::from(current_idx)];
        let cur_state = state[usize::from(current_idx)];

        // Goal test: does this gate exit into the goal sector?
        let (exit_sector, exit_sector_index) = if cur_state.direct {
            (current.sector_in, current.sector_in_index)
        } else {
            (current.sector_out, current.sector_out_index)
        };
        let (entry_sector, entry_sector_index) = if cur_state.direct {
            (current.sector_out, current.sector_out_index)
        } else {
            (current.sector_in, current.sector_in_index)
        };
        let exit_key = GateSectorKey::new(exit_sector, exit_sector_index);
        let entry_key = GateSectorKey::new(entry_sector, entry_sector_index);
        if exit_key == goal_key {
            // `RHFastFindGrid::FindPathGatesNodes` returns the first goal gate
            // removed from the score-ordered open list.  This is also
            // significant when a restored actor's source point is qNaN: the
            // goal gate's score is NaN, but Original still accepts it.
            let mut path: Vec<GatePathStep> = Vec::new();
            let mut step = Some(current_idx);
            while let Some(idx) = step {
                let s = state[usize::from(idx)];
                path.push(GatePathStep {
                    door_index: idx,
                    direct: s.direct,
                });
                step = s.prev_gate;
            }
            path.reverse();
            return Some(path);
        }

        // Expand neighbors via gate links.
        for link in &current.gate_links {
            // RHFastFindGrid selects GetLinkedGates(mbDirect): after
            // traversing the current gate, only links in the sector on the
            // exit side are reachable.
            let link_key = GateSectorKey::new(link.via_sector, link.via_sector_index);
            if link_key != exit_key {
                continue;
            }
            let next_idx = link.other_door;
            if Some(next_idx) == cur_state.prev_gate {
                continue;
            }
            let next = match doors.get(usize::from(next_idx)) {
                Some(d) if d.active => d,
                _ => continue,
            };
            // Authorization check for neighbor gate.
            if let Some(a) = auth {
                let next_direct =
                    GateSectorKey::new(next.sector_out, next.sector_out_index) == link_key;
                if !is_actor_authorized_for_gate(
                    next,
                    next_direct,
                    a,
                    building_has_capacity(next, next_direct, building_is_authorized),
                    allow_leave_map,
                    sector_lift_type,
                ) {
                    continue;
                }
            }

            // Determine direction of next gate based on which side we're entering.
            // We're exiting current via `via_sector` (link.via_sector). The next
            // gate enters from `via_sector` and exits the other side.
            let (next_direct, next_exit_pt, next_entry_pt) =
                if GateSectorKey::new(next.sector_out, next.sector_out_index) == link_key {
                    // We enter next.point_out, exit next.point_in (direct)
                    (true, next.point_in, next.point_out)
                } else if GateSectorKey::new(next.sector_in, next.sector_in_index) == link_key {
                    // We enter next.point_in, exit next.point_out (indirect)
                    (false, next.point_out, next.point_in)
                } else {
                    // Inconsistent link; skip
                    continue;
                };
            // Original: do not use a link that immediately returns to the
            // sector from which the current gate was entered.
            let next_exit_key = if next_direct {
                GateSectorKey::new(next.sector_in, next.sector_in_index)
            } else {
                GateSectorKey::new(next.sector_out, next.sector_out_index)
            };
            if next_exit_key == entry_key {
                continue;
            }

            let new_dist_from_source =
                cur_state.distance_from_source + link.distance + current.penalty;

            let next_state = state[usize::from(next_idx)];
            // Preserve Original's literal
            // `new_distance < previous_distance` test.  Rewriting its
            // negation as `previous_distance <= new_distance` is not
            // equivalent for NaN and repeatedly reopens every gate in a
            // cyclic graph when a restored source position is qNaN.
            if next_state.visited && !(new_dist_from_source < next_state.distance_from_source) {
                continue;
            }

            let d_to_goal = dist(next_exit_pt, goal);
            let new_score = new_dist_from_source + d_to_goal + next.penalty;

            // Use entry point as the link target for distance accounting
            let _ = next_entry_pt;

            state[usize::from(next_idx)] = GateSearchState {
                visited: true,
                direct: next_direct,
                distance_from_source: new_dist_from_source,
                score: new_score,
                prev_gate: Some(current_idx),
            };

            add_to_open_gates_original(&mut open, &state, next_idx);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Penalty constants
// ---------------------------------------------------------------------------

pub const PENALTY_DEFAULT: f32 = 50.0;
pub const PENALTY_JUMP: f32 = 200.0;
pub const PENALTY_BUILDING: f32 = 100.0;

// ---------------------------------------------------------------------------
// Door-targeted gate pathfinding
// ---------------------------------------------------------------------------

/// A* on the gate connectivity graph targeting a specific door.
///
/// The heuristic is `distance-to-door-midpoint`, and the goal-test is
/// identity against `goal_door_index` rather than sector match.
///
/// Returns the full ordered gate list from source to the goal door
/// (inclusive — the last step is `goal_door_index`).  Returns
/// `None` if no path exists or the source sector has no gates.
pub fn find_path_into_door(
    doors: &[Door],
    source: (f32, f32),
    source_sector: u16,
    goal_door_index: DoorIndex,
    auth: Option<&ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<Vec<GatePathStep>> {
    find_path_into_door_with_sector_index(
        doors,
        source,
        source_sector,
        None,
        goal_door_index,
        auth,
        allow_leave_map,
        building_is_authorized,
        sector_lift_type,
    )
}

/// Identity-aware door-targeted gate A*. See
/// [`find_path_gates_with_sector_indices`] for the exact/number-only
/// compatibility rule.
#[allow(clippy::too_many_arguments)]
pub fn find_path_into_door_with_sector_index(
    doors: &[Door],
    source: (f32, f32),
    source_sector: u16,
    source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_door_index: DoorIndex,
    auth: Option<&ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<Vec<GatePathStep>> {
    let source = MapPoint::new(source.0, source.1);
    let source_key =
        GateSectorKey::new(SectorNumber::new(source_sector as i16), source_sector_index);
    let goal_door = doors.get(usize::from(goal_door_index))?;
    // `RHDoor::GetPointMid()` (`original-code/RHGate.h:198`) returns the
    // level-authored `mptMid`, which `RHDoor::InitializeFromProtoStream`
    // deserializes as a point of its own between `point_out` and `point_in`
    // (`original-code/RHGate.cpp:542-578`).  It is NOT the average of
    // `point_in` and `point_out`: on the shipped Leicester door table the two
    // differ on essentially every gate (door 49 is `in=(378,1565)`
    // `out=(362,1535)` `mid=(373,1546)`, not `(370,1550)`).  The A* heuristic
    // in `FindPathIntoDoor` / `FindPathToDoorNodes` measures against the
    // authored mid (`original-code/RHfastfindgrid.cpp:9896`, `:9905`,
    // `:10022`, `:10032`, `:10045`, `:10055`).
    let goal_mid = goal_door.point_mid;

    // No "the goal door already borders the source sector" shortcut:
    // `RHFastFindGrid::FindPathIntoDoor` (`original-code/RHfastfindgrid.cpp:9860`)
    // always seeds every gate of the source sector and runs the A*, and the
    // goal test only fires when the goal door is *popped*
    // (`FindPathToDoorNodes`, `original-code/RHfastfindgrid.cpp:9968`).  A
    // cheaper multi-gate route can therefore relax the goal door first —
    // overwriting its `mbDirect`, `mfDistanceFromSource` and
    // `mpPreviousLink` (`original-code/RHfastfindgrid.cpp:10008-10058`) — so
    // the Original can walk around through neighbouring sectors and arrive at
    // the goal door from its far side even when that door borders the source
    // sector.  Returning a one-step path here skipped those intermediate
    // gates, and with them the building-exit WAIT_TIMER/CHANGE_POSITION
    // sub-sequences (`original-code/RHsequence.cpp:484`, `:519`) and their two
    // `rand() & 15` draws.
    if doors.is_empty() {
        return None;
    }

    let n = doors.len();
    let mut state: Vec<GateSearchState> = vec![GateSearchState::default(); n];
    let mut open: Vec<DoorIndex> = Vec::new();

    // Seed gates touching source sector — heuristic uses goal door
    // mid-point.
    for (idx, door) in doors.iter().enumerate() {
        if !door.active {
            continue;
        }
        if let Some(a) = auth {
            let direct_candidate =
                GateSectorKey::new(door.sector_out, door.sector_out_index) == source_key;
            // Seed pass tests building capacity.
            if !is_actor_authorized_for_gate(
                door,
                direct_candidate,
                a,
                building_has_capacity(door, direct_candidate, building_is_authorized),
                allow_leave_map,
                sector_lift_type,
            ) {
                continue;
            }
        }

        let (direct, from_pt, heuristic_pt) =
            if GateSectorKey::new(door.sector_out, door.sector_out_index) == source_key {
                (true, door.point_out, door.point_in)
            } else if GateSectorKey::new(door.sector_in, door.sector_in_index) == source_key {
                (false, door.point_in, door.point_out)
            } else {
                continue;
            };

        let d_from_src = dist(source, from_pt);
        let d_to_goal = dist(heuristic_pt, goal_mid);
        let score = d_from_src + d_to_goal + door.penalty;

        state[idx] = GateSearchState {
            visited: true,
            direct,
            distance_from_source: d_from_src,
            score,
            prev_gate: None,
        };
        add_to_open_gates_original(&mut open, &state, DoorIndex(idx as u32));
    }

    if open.is_empty() {
        return None;
    }
    // A* main loop.  Goal-test is identity against goal door; heuristic
    // uses distance-to-mid.
    let mut found: Option<DoorIndex> = None;

    while let Some(current_idx) = open.first().copied() {
        open.remove(0);

        if current_idx == goal_door_index {
            found = Some(current_idx);
            break;
        }

        let current = &doors[usize::from(current_idx)];
        let cur_state = state[usize::from(current_idx)];
        let (exit_sector, exit_sector_index) = if cur_state.direct {
            (current.sector_in, current.sector_in_index)
        } else {
            (current.sector_out, current.sector_out_index)
        };
        let (entry_sector, entry_sector_index) = if cur_state.direct {
            (current.sector_out, current.sector_out_index)
        } else {
            (current.sector_in, current.sector_in_index)
        };
        let exit_key = GateSectorKey::new(exit_sector, exit_sector_index);
        let entry_key = GateSectorKey::new(entry_sector, entry_sector_index);
        for link in &current.gate_links {
            // RHFastFindGrid selects GetLinkedGates(mbDirect): after
            // traversing the current gate, only links in the sector on the
            // exit side are reachable.
            let link_key = GateSectorKey::new(link.via_sector, link.via_sector_index);
            if link_key != exit_key {
                continue;
            }
            let next_idx = link.other_door;
            if Some(next_idx) == cur_state.prev_gate {
                continue;
            }
            let next = match doors.get(usize::from(next_idx)) {
                Some(d) if d.active => d,
                _ => continue,
            };
            if let Some(a) = auth {
                let next_direct =
                    GateSectorKey::new(next.sector_out, next.sector_out_index) == link_key;
                if !is_actor_authorized_for_gate(
                    next,
                    next_direct,
                    a,
                    building_has_capacity(next, next_direct, building_is_authorized),
                    allow_leave_map,
                    sector_lift_type,
                ) {
                    continue;
                }
            }

            let (next_direct, next_exit_pt, _next_entry_pt) =
                if GateSectorKey::new(next.sector_out, next.sector_out_index) == link_key {
                    (true, next.point_in, next.point_out)
                } else if GateSectorKey::new(next.sector_in, next.sector_in_index) == link_key {
                    (false, next.point_out, next.point_in)
                } else {
                    continue;
                };
            // Original: do not use a link that immediately returns to the
            // sector from which the current gate was entered.
            let next_exit_key = if next_direct {
                GateSectorKey::new(next.sector_in, next.sector_in_index)
            } else {
                GateSectorKey::new(next.sector_out, next.sector_out_index)
            };
            if next_exit_key == entry_key {
                continue;
            }

            // Penalty is NOT accumulated into g(n) for door-targeted A*
            // (sector-targeted A* does add penalty into g(n)).
            let new_dist_from_source = cur_state.distance_from_source + link.distance;

            let next_state = state[usize::from(next_idx)];
            // Match Original's ordered comparison exactly; see the
            // sector-targeted search above for the NaN-sensitive rationale.
            if next_state.visited && !(new_dist_from_source < next_state.distance_from_source) {
                continue;
            }

            let d_to_goal = dist(next_exit_pt, goal_mid);
            let new_score = new_dist_from_source + d_to_goal + next.penalty;

            state[usize::from(next_idx)] = GateSearchState {
                visited: true,
                direct: next_direct,
                distance_from_source: new_dist_from_source,
                score: new_score,
                prev_gate: Some(current_idx),
            };
            add_to_open_gates_original(&mut open, &state, next_idx);
        }
    }

    let goal_idx = found?;
    let mut path: Vec<GatePathStep> = Vec::new();
    let mut current = Some(goal_idx);
    while let Some(idx) = current {
        let s = state[usize::from(idx)];
        path.push(GatePathStep {
            door_index: idx,
            direct: s.direct,
        });
        current = s.prev_gate;
    }
    path.reverse();
    Some(path)
}

/// Walk-to-door variant: finds a path into `goal_door_index`, pops
/// the goal door off the list, and returns `(gate_path,
/// near_side_point, near_side_sector, near_side_layer)` — the
/// "goal near-side" anchor used when appending a move-to-door step.
///
/// The returned anchor is the side of the goal door the actor
/// approaches from (not the far side of the door): the goal position,
/// sector, and layer are rewritten to the gate's near-side endpoint.
#[allow(clippy::type_complexity)]
pub fn find_path_to_door(
    doors: &[Door],
    source: (f32, f32),
    source_sector: u16,
    goal_door_index: DoorIndex,
    auth: Option<&ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<(Vec<GatePathStep>, (f32, f32), u16, u16)> {
    let full = find_path_into_door(
        doors,
        source,
        source_sector,
        goal_door_index,
        auth,
        allow_leave_map,
        building_is_authorized,
        sector_lift_type,
    )?;
    if full.is_empty() {
        return None;
    }

    let last = *full.last().expect("non-empty path");
    let goal_door = doors.get(usize::from(last.door_index))?;
    let (pt, sector, layer) = if last.direct {
        // Entered from sector_out; approach point is point_out.
        (
            goal_door.point_out,
            u16::from(goal_door.sector_out),
            goal_door.layer_out,
        )
    } else {
        (
            goal_door.point_in,
            u16::from(goal_door.sector_in),
            goal_door.layer_in,
        )
    };

    let mut path = full;
    path.pop();

    Some((path, (pt.x, pt.y), sector, layer))
}

// ---------------------------------------------------------------------------
// Avenger-on-roof wait-position walker
// ---------------------------------------------------------------------------

/// A gate-chain blocking position returned by
/// [`compute_avenger_wait_position`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateWaitPosition {
    pub x: f32,
    pub y: f32,
    pub sector: crate::position_interface::SectorHandle,
    pub layer: u16,
}

/// Park the aggressor short of the gate that blocks its path to the
/// "avenger on the roof".
///
/// Runs `find_path_gates` in the avenger→me direction (so the A* is
/// constrained by gates the *avenger* can traverse), then pops gates
/// from the me-side end of the chain, continuing as long as *the
/// caller* (`me_auth`) can cross each gate in the reverse (me→avenger)
/// direction.  The first gate the caller cannot cross is the one that
/// blocks it; the wait position is that gate's me-side endpoint —
/// i.e. the side the avenger would emerge on.
///
/// Returns `None` when no path exists or when the caller can cross
/// every gate on the path (caller misuse — there is nothing for `me`
/// to wait for).
#[allow(clippy::too_many_arguments)]
pub fn compute_avenger_wait_position(
    doors: &[Door],
    avenger_pos: (f32, f32),
    avenger_sector: crate::position_interface::SectorHandle,
    avenger_auth: &ActorAuthInfo,
    me_pos: (f32, f32),
    me_sector: crate::position_interface::SectorHandle,
    me_auth: &ActorAuthInfo,
    building_is_authorized: &impl Fn(SectorNumber) -> bool,
    sector_lift_type: &impl Fn(SectorNumber) -> Option<LiftType>,
) -> Option<GateWaitPosition> {
    // A* runs avenger → me, gated by the avenger's authorization.
    let path = find_path_gates_with_sector_indices(
        doors,
        avenger_pos,
        avenger_sector.get(),
        avenger_sector.arena_index(),
        me_pos,
        me_sector.get(),
        me_sector.arena_index(),
        Some(avenger_auth),
        false,
        building_is_authorized,
        sector_lift_type,
    )?;
    if path.is_empty() {
        return None;
    }

    // Walk from the me-side end of the chain toward the avenger,
    // popping gates the caller can cross in the reverse direction.
    // Stop at the first unauthorized gate.
    for step in path.iter().rev() {
        let door = doors.get(usize::from(step.door_index))?;
        // `step.direct` is the path direction (avenger→me); me goes
        // the opposite way, so `!step.direct`.
        let reverse_direct = !step.direct;
        if is_actor_authorized_for_gate(
            door,
            reverse_direct,
            me_auth,
            building_has_capacity(door, reverse_direct, building_is_authorized),
            false,
            sector_lift_type,
        ) {
            continue;
        }
        // Blocking gate found.  Wait position is the me-side endpoint
        // along the avenger's forward direction.
        let (pt, sector_number, sector_index, layer) = if step.direct {
            (
                door.point_in,
                u16::from(door.sector_in),
                door.sector_in_index,
                door.layer_in,
            )
        } else {
            (
                door.point_out,
                u16::from(door.sector_out),
                door.sector_out_index,
                door.layer_out,
            )
        };
        let mut sector = crate::position_interface::SectorHandle::new(sector_number)
            .expect("door endpoint sector cannot use the no-sector sentinel");
        if let Some(sector_index) = sector_index {
            sector = sector.with_arena_index(sector_index);
        }
        return Some(GateWaitPosition {
            x: pt.x,
            y: pt.y,
            sector,
            layer,
        });
    }
    None
}

/// Select the doors which define a lift's low/high endpoints, returned as
/// `(lowest, highest)` indices into `doors`.
///
/// Original `RHSectorLift::InitializeFromProtoStream`
/// (`original-code/RHsector.cpp:1493-1521`) owns only the doors whose inside
/// sector is this lift and picks `mpLowestDoor` / `mpHighestDoor` purely by
/// `GetPointOut().mY` — the largest screen Y is the lowest door, the smallest
/// screen Y the highest. Door *type* plays no part: the reference even keeps
/// the `mpHighestDoor->GetType() == DOOR_LIFT_HIGH` assertion commented out
/// (`RHsector.cpp:1524`) because a lift's high endpoint is frequently not a
/// `DOOR_LIFT_HIGH`. Comparisons are strict, so the first door wins a tie.
///
/// Every `RHSectorLift` accessor pair is derived from these two doors:
/// `GetLow/HighExitPoint` → `GetPointIn`, `GetLow/HighEntryPoint` →
/// `GetPointOut`, `GetLow/HighSector`, `GetLow/HighLayer`,
/// `GetLow/HighExitDirection` (`original-code/RHSector.h:315-327`).
pub fn lift_endpoint_door_indices(doors: &[Door], lift_sector: SectorNumber) -> Option<(u32, u32)> {
    let mut candidates = doors
        .iter()
        .enumerate()
        .filter(|(_, door)| door.owning_lift_sector == Some(lift_sector));
    let (first_idx, first) = candidates.next()?;
    let mut low = (first_idx as u32, first.point_out.y);
    let mut high = low;
    for (index, door) in candidates {
        if door.point_out.y > low.1 {
            low = (index as u32, door.point_out.y);
        }
        if door.point_out.y < high.1 {
            high = (index as u32, door.point_out.y);
        }
    }
    assert_ne!(
        low.0, high.0,
        "lift sector {lift_sector} has fewer than two distinct high/low endpoint doors"
    );
    Some((low.0, high.0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn no_lift(_: SectorNumber) -> Option<LiftType> {
        None
    }

    fn sector_handle(number: u16) -> crate::position_interface::SectorHandle {
        crate::position_interface::SectorHandle::new(number).expect("test sector handle")
    }

    #[test]
    fn open_gate_decrease_key_uses_original_live_score_linear_scan() {
        let mut state = vec![GateSearchState::default(); 3];
        state[0].score = 654.0; // Goal gate currently at the front.
        state[1].score = 672.0; // Gate already queued behind it.
        let mut open = vec![DoorIndex(0), DoorIndex(1)];

        // Relaxing gate 1 mutates the shared score in place, making `open`
        // temporarily unsorted: [654, 570]. Original linearly scans that live
        // list and inserts the improved gate before the goal.
        state[1].score = 570.0;
        add_to_open_gates_original(&mut open, &state, DoorIndex(1));
        assert_eq!(open, vec![DoorIndex(1), DoorIndex(0), DoorIndex(1)]);
    }

    #[test]
    fn gate_path_with_nan_source_terminates_and_accepts_first_goal_like_original() {
        // A three-sector cycle plus a goal branch reproduces the topology
        // needed for the schema-16 Savegame_029 replay hang.  Restored
        // Original actors can transiently have qNaN source coordinates;
        // Original then marks the seeded NaN-distance gates visited and does
        // not relax them again because `NaN < NaN` is false.
        let mut doors = vec![
            Door {
                sector_out: SectorNumber::new(1),
                sector_in: SectorNumber::new(2),
                point_out: MapPoint::new(0.0, 0.0),
                point_in: MapPoint::new(10.0, 0.0),
                ..Door::default()
            },
            Door {
                sector_out: SectorNumber::new(2),
                sector_in: SectorNumber::new(3),
                point_out: MapPoint::new(20.0, 0.0),
                point_in: MapPoint::new(30.0, 0.0),
                ..Door::default()
            },
            Door {
                sector_out: SectorNumber::new(3),
                sector_in: SectorNumber::new(1),
                point_out: MapPoint::new(40.0, 0.0),
                point_in: MapPoint::new(50.0, 0.0),
                ..Door::default()
            },
            Door {
                sector_out: SectorNumber::new(3),
                sector_in: SectorNumber::new(4),
                point_out: MapPoint::new(40.0, 10.0),
                point_in: MapPoint::new(50.0, 10.0),
                ..Door::default()
            },
        ];
        build_gate_links(&mut doors);

        let path = find_path_gates(
            &doors,
            (f32::NAN, f32::NAN),
            1,
            (50.0, 10.0),
            4,
            None,
            false,
            &|_| true,
            &no_lift,
        )
        .expect("Original accepts the first goal gate even when its score is NaN");

        assert_eq!(path.last().map(|step| step.door_index), Some(DoorIndex(3)));
    }

    #[test]
    fn exact_gate_graph_does_not_link_distinct_arena_sectors_with_same_number() {
        let sector = |index| crate::fast_find_grid::SectorIndex::new(index).unwrap();
        let mut doors = vec![
            Door {
                sector_out: SectorNumber::new(1),
                sector_in: SectorNumber::new(2),
                sector_out_index: Some(sector(10)),
                sector_in_index: Some(sector(20)),
                point_out: MapPoint::new(0.0, 0.0),
                point_in: MapPoint::new(10.0, 0.0),
                ..Door::default()
            },
            Door {
                // Same public sector 2, but a different runtime object.
                sector_out: SectorNumber::new(2),
                sector_in: SectorNumber::new(3),
                sector_out_index: Some(sector(21)),
                sector_in_index: Some(sector(30)),
                point_out: MapPoint::new(20.0, 0.0),
                point_in: MapPoint::new(30.0, 0.0),
                ..Door::default()
            },
        ];
        build_gate_links(&mut doors);

        assert!(doors.iter().all(|door| door.gate_links.is_empty()));
        assert!(
            find_path_gates_with_sector_indices(
                &doors,
                (0.0, 0.0),
                1,
                Some(sector(10)),
                (30.0, 0.0),
                3,
                Some(sector(30)),
                None,
                false,
                &|_| true,
                &no_lift,
            )
            .is_none()
        );
    }

    #[test]
    fn exact_gate_graph_links_alias_number_only_when_arena_identity_matches() {
        let sector = |index| crate::fast_find_grid::SectorIndex::new(index).unwrap();
        let mut doors = vec![
            Door {
                sector_out: SectorNumber::new(1),
                sector_in: SectorNumber::new(2),
                sector_out_index: Some(sector(10)),
                sector_in_index: Some(sector(20)),
                ..Door::default()
            },
            Door {
                // The public number is intentionally different. Exact arena
                // identity is the Original pointer-equivalent authority.
                sector_out: SectorNumber::new(99),
                sector_in: SectorNumber::new(3),
                sector_out_index: Some(sector(20)),
                sector_in_index: Some(sector(30)),
                ..Door::default()
            },
        ];
        build_gate_links(&mut doors);

        assert_eq!(doors[0].gate_links.len(), 1);
        let path = find_path_gates_with_sector_indices(
            &doors,
            (0.0, 0.0),
            1,
            Some(sector(10)),
            (0.0, 0.0),
            3,
            Some(sector(30)),
            None,
            false,
            &|_| true,
            &no_lift,
        )
        .expect("matching exact identities form a two-gate route");
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn exact_and_number_only_gate_endpoints_never_silently_mix() {
        let sector = |index| crate::fast_find_grid::SectorIndex::new(index).unwrap();
        let mut doors = vec![
            Door {
                sector_out: SectorNumber::new(1),
                sector_in: SectorNumber::new(2),
                sector_out_index: Some(sector(10)),
                sector_in_index: Some(sector(20)),
                ..Door::default()
            },
            Door {
                sector_out: SectorNumber::new(2),
                sector_in: SectorNumber::new(3),
                ..Door::default()
            },
        ];
        build_gate_links(&mut doors);

        assert!(doors.iter().all(|door| door.gate_links.is_empty()));
        assert!(
            find_path_into_door_with_sector_index(
                &doors,
                (0.0, 0.0),
                1,
                Some(sector(10)),
                DoorIndex(1),
                None,
                false,
                &|_| true,
                &no_lift,
            )
            .is_none()
        );
    }

    #[test]
    fn default_door_is_unlocked_and_active() {
        let door = Door::default();
        assert!(door.is_active());
        assert!(door.is_door());
        assert!(!door.is_locked_pc());
        assert!(!door.is_locked_npc_villain());
        assert!(!door.is_locked_npc_civilian());
        assert!(!door.is_unlockable());
        assert!(!door.has_special_authorisation());
    }

    #[test]
    fn click_polygon_includes_original_h02_door_boundary() {
        let mut door = Door::default();
        door.click_polygon = vec![
            (1669.0, 1536.0),
            (1645.0, 1536.0),
            (1644.0, 1485.0),
            (1669.0, 1485.0),
        ];
        door.rebuild_click_bbox();

        assert!(door.click_polygon_contains(1669.0, 1505.44));
        assert!(door.click_polygon_contains(1655.0, 1505.44));
        assert!(!door.click_polygon_contains(1670.0, 1505.44));
    }

    #[test]
    fn lock_unlock_pc() {
        let mut door = Door::default();
        door.lock_pc();
        assert!(door.is_locked_pc());
        door.unlock_pc();
        assert!(!door.is_locked_pc());
    }

    #[test]
    fn lock_unlock_npc_villain() {
        let mut door = Door::default();
        door.lock_npc_villain();
        assert!(door.is_locked_npc_villain());
        door.unlock_npc_villain();
        assert!(!door.is_locked_npc_villain());
    }

    #[test]
    fn lock_unlock_npc_civilian() {
        let mut door = Door::default();
        door.lock_npc_civilian();
        assert!(door.is_locked_npc_civilian());
        door.unlock_npc_civilian();
        assert!(!door.is_locked_npc_civilian());
    }

    #[test]
    fn set_locked_variants() {
        let mut door = Door::default();
        door.set_locked_pc(true);
        door.set_locked_npc_villain(true);
        door.set_locked_npc_civilian(true);
        assert!(door.is_locked_pc());
        assert!(door.is_locked_npc_villain());
        assert!(door.is_locked_npc_civilian());

        door.set_locked_pc(false);
        door.set_locked_npc_villain(false);
        door.set_locked_npc_civilian(false);
        assert!(!door.is_locked_pc());
        assert!(!door.is_locked_npc_villain());
        assert!(!door.is_locked_npc_civilian());
    }

    #[test]
    fn unlockable_flag() {
        let mut door = Door::default();
        door.set_unlockable(true);
        assert!(door.is_unlockable());
        door.set_unlockable(false);
        assert!(!door.is_unlockable());
    }

    #[test]
    fn active_flag() {
        let mut door = Door::default();
        assert!(door.is_active());
        door.set_active(false);
        assert!(!door.is_active());
    }

    #[test]
    fn special_authorisation_requires_locked_pc() {
        let mut door = Door {
            special_authorisation_pc: true,
            ..Default::default()
        };
        // Not locked for PC → has_special_authorisation should be false.
        assert!(!door.has_special_authorisation());

        door.lock_pc();
        assert!(door.has_special_authorisation());
    }

    #[test]
    fn grant_and_check_special_authorisation() {
        let mut door = Door::default();
        door.lock_pc();

        let robin_bit: u16 = 0b0001;
        let marian_bit: u16 = 0b0010;

        door.grant_special_authorisation(robin_bit, true);
        door.grant_special_authorisation(marian_bit, false);

        assert!(door.has_special_authorisation_for(robin_bit, true));
        assert!(!door.has_special_authorisation_for(robin_bit, false));
        assert!(!door.has_special_authorisation_for(marian_bit, true));
        assert!(door.has_special_authorisation_for(marian_bit, false));
    }

    #[test]
    fn swap_rights_patch() {
        let mut door = Door {
            locked_pc: false,
            locked_npc_villain: false,
            locked_npc_civilian: true,
            unlockable: false,
            locked_pc_after_patch: true,
            locked_npc_villain_after_patch: true,
            locked_npc_civilian_after_patch: false,
            unlockable_after_patch: true,
            ..Default::default()
        };

        door.swap_rights_patch();

        assert!(door.locked_pc);
        assert!(door.locked_npc_villain);
        assert!(!door.locked_npc_civilian);
        assert!(door.unlockable);

        // The old values should now be in the "after patch" slots.
        assert!(!door.locked_pc_after_patch);
        assert!(!door.locked_npc_villain_after_patch);
        assert!(door.locked_npc_civilian_after_patch);
        assert!(!door.unlockable_after_patch);
    }

    #[test]
    fn double_swap_restores_original() {
        let mut door = Door {
            locked_pc: true,
            locked_npc_villain: false,
            locked_npc_civilian: true,
            unlockable: true,
            locked_pc_after_patch: false,
            locked_npc_villain_after_patch: true,
            locked_npc_civilian_after_patch: false,
            unlockable_after_patch: false,
            ..Default::default()
        };

        door.swap_rights_patch();
        door.swap_rights_patch();

        assert!(door.locked_pc);
        assert!(!door.locked_npc_villain);
        assert!(door.locked_npc_civilian);
        assert!(door.unlockable);
    }

    #[test]
    fn geometry_setters() {
        let mut door = Door::default();
        door.set_point_out(10.0, 20.0);
        door.set_point_in(30.0, 40.0);
        door.set_point_mid(20.0, 30.0);

        assert_eq!(door.point_out(), MapPoint::new(10.0, 20.0));
        assert_eq!(door.point_in(), MapPoint::new(30.0, 40.0));
        assert_eq!(door.point_mid(), MapPoint::new(20.0, 30.0));
    }

    #[test]
    fn serde_roundtrip() {
        let mut door = Door::default();
        door.lock_pc();
        door.lock_npc_civilian();
        door.set_unlockable(true);
        door.special_authorisation_pc = true;
        door.authorised_pc_direct = 0b0101;
        door.authorised_pc_indirect = 0b1010;
        door.door_type = DoorType::Building;

        let json = serde_json::to_string(&door).unwrap();
        let door2: Door = serde_json::from_str(&json).unwrap();

        assert_eq!(door.locked_pc, door2.locked_pc);
        assert_eq!(door.locked_npc_villain, door2.locked_npc_villain);
        assert_eq!(door.locked_npc_civilian, door2.locked_npc_civilian);
        assert_eq!(door.unlockable, door2.unlockable);
        assert_eq!(
            door.special_authorisation_pc,
            door2.special_authorisation_pc
        );
        assert_eq!(door.authorised_pc_direct, door2.authorised_pc_direct);
        assert_eq!(door.authorised_pc_indirect, door2.authorised_pc_indirect);
        assert_eq!(door.gate_type, door2.gate_type);
        assert_eq!(door.door_type, door2.door_type);
        assert_eq!(door.active, door2.active);

        // Skipped fields should be at defaults after deserialization.
        assert_eq!(door2.point_out, MapPoint::ZERO);
        assert_eq!(door2.penalty, 0.0);
    }

    // -- get_action_1 / get_action_2 --

    fn door_with_type(door_type: DoorType) -> Door {
        let (d1, d2, i1, i2) = Door::default_actions_for_type(door_type);
        Door {
            door_type,
            action_direct_1: d1,
            action_direct_2: d2,
            action_indirect_1: i1,
            action_indirect_2: i2,
            ..Default::default()
        }
    }

    #[test]
    fn get_action_default_door_runs_when_requested_run() {
        // Plain door: per-door hint is WalkingUpright, request RunningUpright.
        // Returns RunningUpright (door doesn't slow you down).
        let door = door_with_type(DoorType::Default);
        assert_eq!(
            door.get_action_1(true, OrderType::RunningUpright),
            OrderType::RunningUpright
        );
        assert_eq!(
            door.get_action_2(false, OrderType::RunningUpright),
            OrderType::RunningUpright
        );
    }

    #[test]
    fn get_action_default_door_walking_passes_through() {
        let door = door_with_type(DoorType::Default);
        assert_eq!(
            door.get_action_1(true, OrderType::WalkingUpright),
            OrderType::WalkingUpright
        );
    }

    #[test]
    fn get_action_default_or_gate_preserves_walking_crouched() {
        for ty in [DoorType::Default, DoorType::Gate] {
            let door = door_with_type(ty);
            assert_eq!(
                door.get_action_1(true, OrderType::WalkingCrouched),
                OrderType::WalkingCrouched
            );
            assert_eq!(
                door.get_action_2(false, OrderType::WalkingCrouched),
                OrderType::WalkingCrouched
            );
        }
    }

    #[test]
    fn get_action_stairs_demotes_running_to_walking_stairs() {
        // Synthesize a door whose direct_1 hint is WalkingStairs (the
        // value that stairs-typed lifts install).  Running stays as
        // running because the hint is in the "walking" allow-list.
        let mut door = door_with_type(DoorType::LiftHigh);
        door.action_direct_1 = OrderType::WalkingStairs;
        door.action_direct_2 = OrderType::WalkingStairs;
        // RunningUpright + WalkingStairs hint → returns RunningUpright
        assert_eq!(
            door.get_action_1(true, OrderType::RunningUpright),
            OrderType::RunningUpright
        );
        // WalkingUpright + WalkingStairs hint → returns hint (stairs walk)
        assert_eq!(
            door.get_action_1(true, OrderType::WalkingUpright),
            OrderType::WalkingStairs
        );
    }

    #[test]
    fn get_action_trap_door_substitutes_ladder_transitions() {
        let door = door_with_type(DoorType::BuildingTrap);
        // Direct entry: action_direct_1 = WalkingUpright (so request wins),
        // but the exit step (action_direct_2 = LADDER_DOWN) is NOT in the
        // walking allow-list, so the hint always replaces the request.
        assert_eq!(
            door.get_action_1(true, OrderType::WalkingUpright),
            OrderType::WalkingUpright
        );
        assert_eq!(
            door.get_action_2(true, OrderType::WalkingUpright),
            OrderType::TransitionWaitingCrouchedClimbingLadderDown
        );
        assert_eq!(
            door.get_action_2(true, OrderType::RunningUpright),
            OrderType::TransitionWaitingCrouchedClimbingLadderDown
        );
        // Indirect (inside → outside): action_indirect_1 hint is the
        // ladder-up transition.
        assert_eq!(
            door.get_action_1(false, OrderType::WalkingUpright),
            OrderType::TransitionClimbingLadderUpWaitingCrouched
        );
    }

    #[test]
    fn get_action_trap_door_for_plain_trap_too() {
        // Trap shares the same default actions as BuildingTrap.
        let door = door_with_type(DoorType::Trap);
        assert_eq!(
            door.get_action_2(true, OrderType::WalkingUpright),
            OrderType::TransitionWaitingCrouchedClimbingLadderDown
        );
    }

    #[test]
    fn default_actions_for_type_matches_door_kind() {
        // Default, Building, Gate: all WalkingUpright.
        for ty in [DoorType::Default, DoorType::Building, DoorType::Gate] {
            let (d1, d2, i1, i2) = Door::default_actions_for_type(ty);
            assert_eq!(d1, OrderType::WalkingUpright);
            assert_eq!(d2, OrderType::WalkingUpright);
            assert_eq!(i1, OrderType::WalkingUpright);
            assert_eq!(i2, OrderType::WalkingUpright);
        }
        // Trap and BuildingTrap get the ladder-transition pair.
        for ty in [DoorType::Trap, DoorType::BuildingTrap] {
            let (d1, d2, i1, i2) = Door::default_actions_for_type(ty);
            assert_eq!(d1, OrderType::WalkingUpright);
            assert_eq!(d2, OrderType::TransitionWaitingCrouchedClimbingLadderDown);
            assert_eq!(i1, OrderType::TransitionClimbingLadderUpWaitingCrouched);
            assert_eq!(i2, OrderType::WalkingUpright);
        }
    }

    #[test]
    fn door_type_enum_variants() {
        // Ensure all variants exist.
        let types = [
            DoorType::Default,
            DoorType::Building,
            DoorType::BuildingTrap,
            DoorType::Gate,
            DoorType::LiftHigh,
            DoorType::LiftLow,
            DoorType::LiftHighCrenel,
            DoorType::Trap,
            DoorType::Reinforcement,
        ];
        assert_eq!(types.len(), 9);
    }

    #[test]
    fn gate_type_enum_variants() {
        let types = [GateType::None, GateType::Door, GateType::Jump];
        assert_eq!(types.len(), 3);
        assert_eq!(GateType::default(), GateType::None);
    }

    #[test]
    fn penalty_constants() {
        assert_eq!(PENALTY_DEFAULT, 50.0);
        assert_eq!(PENALTY_JUMP, 200.0);
        assert_eq!(PENALTY_BUILDING, 100.0);
    }

    // -- Gate state machine tests --

    #[test]
    fn gate_state_default_is_closed() {
        assert_eq!(GateState::default(), GateState::Closed);
        assert!(!GateState::Closed.is_passable());
        assert!(GateState::Open.is_passable());
    }

    #[test]
    fn gate_state_open_close_cycle() {
        let mut state = GateState::Closed;
        state.request_open();
        assert_eq!(state, GateState::Opening);
        assert!(!state.is_passable());

        state.finish_transition();
        assert_eq!(state, GateState::Open);
        assert!(state.is_passable());

        state.request_close();
        assert_eq!(state, GateState::Closing);
        state.finish_transition();
        assert_eq!(state, GateState::Closed);
    }

    #[test]
    fn gate_state_toggle() {
        let mut state = GateState::Closed;
        state.toggle();
        assert_eq!(state, GateState::Opening);
        state.toggle();
        assert_eq!(state, GateState::Closing);
        state.toggle();
        assert_eq!(state, GateState::Opening);
        state.finish_transition();
        assert_eq!(state, GateState::Open);
        state.toggle();
        assert_eq!(state, GateState::Closing);
    }

    #[test]
    fn gate_state_redundant_requests_are_noop() {
        let mut state = GateState::Open;
        state.request_open(); // already open
        assert_eq!(state, GateState::Open);

        let mut state = GateState::Closed;
        state.request_close(); // already closed
        assert_eq!(state, GateState::Closed);
    }

    // -- Door authorization tests --

    fn pc_actor(has_lockpick: bool) -> ActorAuthInfo {
        ActorAuthInfo {
            kind: ElementKind::ActorPc,
            pc_auth_bit: 0x0001,
            has_lockpick,
            has_climb: false,
            has_jump: false,
            is_rider: false,
            posture: crate::element::Posture::Upright,
        }
    }

    fn soldier_actor(is_rider: bool) -> ActorAuthInfo {
        ActorAuthInfo {
            kind: ElementKind::ActorSoldier,
            pc_auth_bit: 0,
            has_lockpick: false,
            has_climb: false,
            has_jump: false,
            is_rider,
            posture: crate::element::Posture::Upright,
        }
    }

    fn civilian_actor() -> ActorAuthInfo {
        ActorAuthInfo {
            kind: ElementKind::ActorCivilian,
            pc_auth_bit: 0,
            has_lockpick: false,
            has_climb: false,
            has_jump: false,
            is_rider: false,
            posture: crate::element::Posture::Upright,
        }
    }

    #[test]
    fn auth_unlocked_default_door_allows_everyone() {
        let door = Door::default();
        assert!(door.is_actor_authorized(true, &pc_actor(false), true, false));
        assert!(door.is_actor_authorized(true, &soldier_actor(false), true, false));
        assert!(door.is_actor_authorized(true, &civilian_actor(), true, false));
    }

    #[test]
    fn auth_locked_pc_door_blocks_pc_without_lockpick() {
        let mut door = Door::default();
        door.lock_pc();
        assert!(!door.is_actor_authorized(true, &pc_actor(false), true, false));
    }

    #[test]
    fn auth_locked_pc_unlockable_door_allows_pc_with_lockpick() {
        let mut door = Door::default();
        door.lock_pc();
        door.set_unlockable(true);
        assert!(door.is_actor_authorized(true, &pc_actor(true), true, false));
    }

    #[test]
    fn auth_locked_pc_non_unlockable_blocks_even_with_lockpick() {
        let mut door = Door::default();
        door.lock_pc();
        // unlockable is false by default
        assert!(!door.is_actor_authorized(true, &pc_actor(true), true, false));
    }

    #[test]
    fn auth_locked_npc_villain_blocks_soldiers() {
        let mut door = Door::default();
        door.lock_npc_villain();
        assert!(!door.is_actor_authorized(true, &soldier_actor(false), true, false));
        // Civilians unaffected
        assert!(door.is_actor_authorized(true, &civilian_actor(), true, false));
    }

    #[test]
    fn auth_locked_npc_civilian_blocks_civilians() {
        let mut door = Door::default();
        door.lock_npc_civilian();
        assert!(!door.is_actor_authorized(true, &civilian_actor(), true, false));
        // Soldiers unaffected
        assert!(door.is_actor_authorized(true, &soldier_actor(false), true, false));
    }

    #[test]
    fn auth_building_door_blocks_rider_soldiers() {
        let door = Door {
            door_type: DoorType::Building,
            ..Default::default()
        };
        assert!(!door.is_actor_authorized(true, &soldier_actor(true), true, false));
    }

    #[test]
    fn auth_building_door_blocks_npc_when_full() {
        let door = Door {
            door_type: DoorType::Building,
            ..Default::default()
        };
        // building_has_capacity = false
        assert!(!door.is_actor_authorized(true, &soldier_actor(false), false, false));
        // PC still allowed even when full
        assert!(door.is_actor_authorized(true, &pc_actor(false), false, false));
    }

    #[test]
    fn gate_path_blocks_direct_npc_entry_into_full_building() {
        let mut doors = vec![
            Door {
                sector_out: SectorNumber::new(1),
                sector_in: SectorNumber::new(2),
                point_out: MapPoint::new(0.0, 0.0),
                point_in: MapPoint::new(10.0, 0.0),
                ..Door::default()
            },
            Door {
                door_type: DoorType::Building,
                sector_out: SectorNumber::new(2),
                sector_in: SectorNumber::new(3),
                point_out: MapPoint::new(20.0, 0.0),
                point_in: MapPoint::new(30.0, 0.0),
                ..Door::default()
            },
        ];
        build_gate_links(&mut doors);
        let soldier = soldier_actor(false);

        let full = |sector: SectorNumber| sector != SectorNumber::new(3);
        assert!(
            find_path_gates(
                &doors,
                (0.0, 0.0),
                1,
                (30.0, 0.0),
                3,
                Some(&soldier),
                false,
                &full,
                &no_lift,
            )
            .is_none(),
            "neighbor expansion must reject direct entry into a full building"
        );
        assert!(
            find_path_gates(
                &doors,
                (20.0, 0.0),
                2,
                (30.0, 0.0),
                3,
                Some(&soldier),
                false,
                &full,
                &no_lift,
            )
            .is_none(),
            "seed gates must reject direct entry into a full building"
        );

        let available = |_: SectorNumber| true;
        let path = find_path_gates(
            &doors,
            (0.0, 0.0),
            1,
            (30.0, 0.0),
            3,
            Some(&soldier),
            false,
            &available,
            &no_lift,
        )
        .expect("the same route is valid while the building has capacity");
        assert_eq!(path.len(), 2);
        assert!(path.iter().all(|step| step.direct));
    }

    #[test]
    fn auth_building_trap_never_allows_civilians() {
        let door = Door {
            door_type: DoorType::BuildingTrap,
            ..Default::default()
        };
        assert!(!door.is_actor_authorized(true, &civilian_actor(), true, false));
    }

    #[test]
    fn auth_reinforcement_door_blocked_unless_allow_leave_map() {
        let door = Door {
            door_type: DoorType::Reinforcement,
            ..Default::default()
        };
        assert!(!door.is_actor_authorized(true, &pc_actor(false), true, false));
        assert!(door.is_actor_authorized(true, &pc_actor(false), true, true));
    }

    #[test]
    fn auth_inactive_door_blocks_everyone() {
        let mut door = Door::default();
        door.set_active(false);
        assert!(!door.is_actor_authorized(true, &pc_actor(false), true, false));
        assert!(!door.is_actor_authorized(true, &soldier_actor(false), true, false));
    }

    #[test]
    fn auth_special_authorisation_overrides_lock() {
        let mut door = Door::default();
        door.lock_pc();
        door.grant_special_authorisation(0x0001, true);
        let cooper = pc_actor(false);
        // Cooper authorized in direct direction
        assert!(door.is_actor_authorized(true, &cooper, true, false));
        // Not authorized in indirect direction
        assert!(!door.is_actor_authorized(false, &cooper, true, false));
    }

    #[test]
    fn auth_lift_door_blocks_riders() {
        let door = Door {
            door_type: DoorType::LiftHigh,
            ..Default::default()
        };
        assert!(!door.is_actor_authorized(true, &soldier_actor(true), true, false));
        // Non-rider allowed (lift type check is caller's responsibility)
        assert!(door.is_actor_authorized(true, &soldier_actor(false), true, false));
    }

    #[test]
    fn auth_jump_gate_requires_pc_and_has_jump() {
        let door = Door {
            gate_type: GateType::Jump,
            ..Default::default()
        };
        // Non-PC never authorized.
        assert!(!door.is_actor_authorized(true, &soldier_actor(false), true, false));
        assert!(!door.is_actor_authorized(true, &civilian_actor(), true, false));
        // PC without jump action never authorized.
        let mut pc_no_jump = pc_actor(false);
        pc_no_jump.has_jump = false;
        assert!(!door.is_actor_authorized(true, &pc_no_jump, true, false));
    }

    #[test]
    fn auth_jump_gate_no_helper_needed_allows_any_posture() {
        let door = Door {
            gate_type: GateType::Jump,
            jump_line_in_helper_needed: false,
            jump_line_out_helper_needed: false,
            ..Default::default()
        };
        let mut pc = pc_actor(false);
        pc.has_jump = true;
        pc.posture = crate::element::Posture::Upright;
        assert!(door.is_actor_authorized(true, &pc, true, false));
        assert!(door.is_actor_authorized(false, &pc, true, false));
    }

    #[test]
    fn auth_jump_gate_helper_needed_checks_posture_per_direction() {
        // direct destination (in-side) needs helper; indirect does not.
        let door = Door {
            gate_type: GateType::Jump,
            jump_line_in_helper_needed: true,
            jump_line_out_helper_needed: false,
            ..Default::default()
        };
        let mut pc = pc_actor(false);
        pc.has_jump = true;

        // Upright PC: direct (helper needed) rejected; indirect (helper
        // not needed) allowed.
        pc.posture = crate::element::Posture::Upright;
        assert!(!door.is_actor_authorized(true, &pc, true, false));
        assert!(door.is_actor_authorized(false, &pc, true, false));

        // On-shoulders PC: both directions allowed.
        pc.posture = crate::element::Posture::OnShoulders;
        assert!(door.is_actor_authorized(true, &pc, true, false));
        assert!(door.is_actor_authorized(false, &pc, true, false));
    }

    #[test]
    fn adapt_points_building_trap_offsets_point_in_by_60() {
        // point_in is 100 units east of point_mid.
        let mut door = Door {
            door_type: DoorType::BuildingTrap,
            point_in: MapPoint::new(100.0, 0.0),
            point_mid: MapPoint::ZERO,
            ..Default::default()
        };
        door.adapt_points(false);
        // Shifted to 60 units east of mid.
        assert!((door.point_in.x - 60.0).abs() < 1e-4);
        assert!(door.point_in.y.abs() < 1e-4);
    }

    #[test]
    fn adapt_points_lift_high_crenel_wall_uses_65() {
        let mut door = Door {
            door_type: DoorType::LiftHighCrenel,
            point_in: MapPoint::new(0.0, 100.0),
            point_mid: MapPoint::ZERO,
            ..Default::default()
        };
        door.adapt_points(true);
        assert!(door.point_in.x.abs() < 1e-4);
        assert!((door.point_in.y - 65.0).abs() < 1e-4);
    }

    #[test]
    fn adapt_points_lift_high_ignores_non_wall_lift() {
        // LiftHigh only shifts when lift_wall = true.
        let mut door = Door {
            door_type: DoorType::LiftHigh,
            point_in: MapPoint::new(100.0, 0.0),
            point_mid: MapPoint::ZERO,
            ..Default::default()
        };
        door.adapt_points(false);
        assert_eq!(door.point_in, MapPoint::new(100.0, 0.0));
    }

    #[test]
    fn adapt_points_default_door_is_noop() {
        let mut door = Door {
            door_type: DoorType::Default,
            point_in: MapPoint::new(100.0, 100.0),
            point_mid: MapPoint::new(50.0, 50.0),
            ..Default::default()
        };
        door.adapt_points(true);
        assert_eq!(door.point_in, MapPoint::new(100.0, 100.0));
    }

    #[test]
    fn adapt_points_zero_length_is_noop() {
        // point_in == point_mid → normalize would divide by zero.
        let mut door = Door {
            door_type: DoorType::BuildingTrap,
            point_in: MapPoint::new(42.0, 13.0),
            point_mid: MapPoint::new(42.0, 13.0),
            ..Default::default()
        };
        door.adapt_points(false);
        assert_eq!(door.point_in, MapPoint::new(42.0, 13.0));
    }

    #[test]
    fn compute_door_penalty_default_adds_50() {
        let mut door = Door {
            door_type: DoorType::Default,
            point_in: MapPoint::new(100.0, 0.0),
            point_out: MapPoint::ZERO,
            ..Default::default()
        };
        door.compute_door_penalty();
        assert!((door.penalty - (100.0 + PENALTY_DEFAULT)).abs() < 1e-4);
    }

    #[test]
    fn compute_door_penalty_building_adds_100() {
        for ty in [DoorType::Building, DoorType::BuildingTrap] {
            let mut door = Door {
                door_type: ty,
                point_in: MapPoint::new(0.0, 100.0),
                point_out: MapPoint::ZERO,
                ..Default::default()
            };
            door.compute_door_penalty();
            assert!((door.penalty - (100.0 + PENALTY_BUILDING)).abs() < 1e-4);
        }
    }

    #[test]
    fn body_would_block_door() {
        let door = Door::default();
        let sn = crate::sector::SectorNumber::new;
        // Just inside the threshold (400) — blocks.
        assert!(door.body_would_block(sn(1), sn(1), sn(2), 399.0, 5000.0));
        // Just outside — does not block.
        assert!(!door.body_would_block(sn(1), sn(1), sn(2), 401.0, 5000.0));
        // Exactly at threshold counts as blocking (`<=`).
        assert!(door.body_would_block(sn(1), sn(1), sn(2), 400.0, 5000.0));
        // Close to the out-point in the out-sector
        assert!(door.body_would_block(sn(2), sn(1), sn(2), 5000.0, 399.0));
        // Too far from both
        assert!(!door.body_would_block(sn(1), sn(1), sn(2), 1000.0, 5000.0));
        // Wrong sector
        assert!(!door.body_would_block(sn(3), sn(1), sn(2), 399.0, 399.0));
    }

    // -- Avenger wait-position walker --

    /// Two-sector map: avenger in sector 2, me in sector 1, connected
    /// by door locked for NPC-civilians only.  The avenger (a PC) can
    /// cross; me (a civilian) cannot.  Wait position = door's me-side
    /// endpoint.
    #[test]
    fn avenger_wait_position_blocked_by_civilian_lock() {
        let mut door = Door {
            sector_out: crate::sector::SectorNumber::new(1), // me's sector
            sector_in: crate::sector::SectorNumber::new(2),  // avenger's sector
            point_out: MapPoint::new(100.0, 100.0),
            point_in: MapPoint::new(100.0, 200.0),
            layer_out: 0,
            layer_in: 0,
            ..Door::default()
        };
        door.lock_npc_civilian();
        let doors = vec![door];

        let avenger = pc_actor(false);
        let me = civilian_actor();

        // Avenger at (100,300) in sector 2, me at (100,0) in sector 1.
        let wait = compute_avenger_wait_position(
            &doors,
            (100.0, 300.0),
            sector_handle(2),
            &avenger,
            (100.0, 0.0),
            sector_handle(1),
            &me,
            &|_| true,
            &no_lift,
        )
        .expect("path exists, me blocked by civilian lock");

        // Path direction avenger→me is in→out (door.sector_in=2=source,
        // door.sector_out=1=goal), so step.direct is false.
        // Wait position = GetPositionOut = (100, 100) in sector 1.
        assert_eq!(wait.x, 100.0);
        assert_eq!(wait.y, 100.0);
        assert_eq!(wait.sector.get(), 1);
    }

    /// Path direction avenger→me goes out→in (step.direct = true);
    /// the wait-position should be the door's point_in (me-side).
    #[test]
    fn avenger_wait_position_direct_step_uses_point_in() {
        let mut door = Door {
            sector_out: crate::sector::SectorNumber::new(2), // avenger's sector
            sector_in: crate::sector::SectorNumber::new(1),  // me's sector
            point_out: MapPoint::new(100.0, 200.0),
            point_in: MapPoint::new(100.0, 100.0),
            layer_out: 0,
            layer_in: 0,
            ..Door::default()
        };
        door.lock_npc_civilian();
        let doors = vec![door];

        let avenger = pc_actor(false);
        let me = civilian_actor();

        let wait = compute_avenger_wait_position(
            &doors,
            (100.0, 300.0),
            sector_handle(2),
            &avenger,
            (100.0, 0.0),
            sector_handle(1),
            &me,
            &|_| true,
            &no_lift,
        )
        .expect("path exists, me blocked by civilian lock");

        assert_eq!(wait.x, 100.0);
        assert_eq!(wait.y, 100.0);
        assert_eq!(wait.sector.get(), 1);
    }

    /// The reverse me→avenger scan must apply the same live building
    /// capacity check as `RHDoor::IsActorAutorized`. The avenger can leave a
    /// full building through the near door, while the waiting soldier cannot
    /// enter it; once capacity becomes available, the later soldier lock is
    /// the first blocker instead.
    #[test]
    fn avenger_wait_position_reverse_scan_observes_building_capacity() {
        let building = Door {
            door_type: DoorType::Building,
            sector_out: SectorNumber::new(1), // me's sector
            sector_in: SectorNumber::new(2),
            point_out: MapPoint::new(10.0, 0.0),
            point_in: MapPoint::new(20.0, 0.0),
            ..Door::default()
        };
        let mut later_blocker = Door {
            sector_out: SectorNumber::new(2),
            sector_in: SectorNumber::new(3), // avenger's sector
            point_out: MapPoint::new(30.0, 0.0),
            point_in: MapPoint::new(40.0, 0.0),
            ..Door::default()
        };
        later_blocker.lock_npc_villain();
        let mut doors = vec![building, later_blocker];
        build_gate_links(&mut doors);

        let avenger = pc_actor(false);
        let me = soldier_actor(false);
        let full = |sector: SectorNumber| sector != SectorNumber::new(2);
        let wait = compute_avenger_wait_position(
            &doors,
            (40.0, 0.0),
            sector_handle(3),
            &avenger,
            (0.0, 0.0),
            sector_handle(1),
            &me,
            &full,
            &no_lift,
        )
        .expect("full building is the first reverse-direction blocker");
        assert_eq!(wait.x, 10.0);
        assert_eq!(wait.y, 0.0);
        assert_eq!(wait.sector.get(), 1);

        let available = |_: SectorNumber| true;
        let wait = compute_avenger_wait_position(
            &doors,
            (40.0, 0.0),
            sector_handle(3),
            &avenger,
            (0.0, 0.0),
            sector_handle(1),
            &me,
            &available,
            &no_lift,
        )
        .expect("later soldier lock blocks once the building has capacity");
        assert_eq!(wait.x, 30.0);
        assert_eq!(wait.y, 0.0);
        assert_eq!(wait.sector.get(), 2);
    }

    /// Me can pass every gate on the path — caller-misuse case.
    /// Returns `None` rather than fabricating a position.
    #[test]
    fn avenger_wait_position_returns_none_when_me_can_pass() {
        let door = Door {
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in: crate::sector::SectorNumber::new(2),
            point_out: MapPoint::new(100.0, 100.0),
            point_in: MapPoint::new(100.0, 200.0),
            ..Door::default()
        };
        let doors = vec![door];

        let avenger = pc_actor(false);
        let me = soldier_actor(false);

        let wait = compute_avenger_wait_position(
            &doors,
            (100.0, 300.0),
            sector_handle(2),
            &avenger,
            (100.0, 0.0),
            sector_handle(1),
            &me,
            &|_| true,
            &no_lift,
        );
        assert!(wait.is_none());
    }

    /// No path exists (avenger is sealed off) — returns None.
    #[test]
    fn avenger_wait_position_returns_none_when_no_path() {
        let doors: Vec<Door> = Vec::new();

        let avenger = pc_actor(false);
        let me = soldier_actor(false);

        let wait = compute_avenger_wait_position(
            &doors,
            (100.0, 300.0),
            sector_handle(2),
            &avenger,
            (100.0, 0.0),
            sector_handle(1),
            &me,
            &|_| true,
            &no_lift,
        );
        assert!(wait.is_none());
    }
}
