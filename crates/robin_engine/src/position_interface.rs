//! Position interface — position, movement, direction, and collision for entities.
//!
//! Every mobile game entity owns a `PositionInterface` that tracks its 3D
//! position, 2D map position, sprite position, movement direction (16
//! sectors), increment vectors, move bounding box, anti-collision state, and
//! more.
//!
//! The position system uses **eager computation**: every `set_position_*`
//! setter writes the authoritative field and immediately recomputes the
//! other coordinate systems so all three stay in sync.  Increment vectors
//! still use lazy derivation via `compute_increment_*`.
//!
//! Actor-vs-actor anti-collision ships via
//! [`PositionInterface::update_position_anti_collision`] and the free
//! function [`compute_deviated_future`] — the engine's tick loop
//! gathers neighbour repulsive points (see `engine::anti_collision`)
//! and pushes moving actors around each other.  Level-authored
//! repulsive-line / repulsive-point grid buckets are ported via
//! `FastFindGrid::get_active_repulsive_line_indices` and
//! `engine::anti_collision::gather_static_repulsive_points`.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::coordinates::{MapBBox, MapPoint, MapVec, WorldPoint3D, WorldVec3D};
use crate::fast_find_grid::{FastFindGrid, GRID_CELL_SIZE};
use crate::geo2d::{self, BBox2D, Vec2D};
use crate::repulsive::{RepulsiveLine, RepulsivePoint};

// ---------------------------------------------------------------------------
// Plane Z-coefficients
// ---------------------------------------------------------------------------

/// Z-computation coefficients: `z = az·x + bz·y + dz`.
///
/// The full plane lives on the sight obstacle; we cache only the
/// coefficients needed by the position math here.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct PlaneZCoeffs {
    pub az: f32,
    pub bz: f32,
    pub dz: f32,
}

impl PlaneZCoeffs {
    /// Compute Z given map-space `(x, y)`.
    ///
    /// `z = (bz * y + az * x + dz) / (1 - bz)`
    #[inline]
    pub fn compute_z(&self, x: f32, y: f32) -> f32 {
        (self.bz * y + self.az * x + self.dz) / (1.0 - self.bz)
    }

    /// Compute Z-increment for a map-space movement `(dx, dy)`.
    #[inline]
    pub fn compute_z_increment(&self, dx: f32, dy: f32) -> f32 {
        (self.bz * dy + self.az * dx) / (1.0 - self.bz)
    }

    /// Derive the iso-corrected coefficients from three world-space
    /// plane points (e.g. a sight obstacle's `top_plane_points`).
    ///
    /// The world plane is `z = a·x + b·y + d` with
    /// `a = -nx/nz`, `b = -ny/nz`, `d = p0.z + (nx·p0.x + ny·p0.y)/nz`,
    /// where `(nx, ny, nz) = (p1-p0) × (p2-p0)`.  Stored in
    /// `(az, bz, dz)`; [`Self::compute_z`] applies the iso correction
    /// `(1 - bz)` divisor when projecting from screen-Y back to world Z.
    /// Degenerate (vertical) planes collapse to the average Z of the
    /// three input points.
    /// Resolve the top-plane coefficients for an `ObstacleHandle` by
    /// indexing into a flat `SightObstacle` slice.  Returns `None` when
    /// `obs` is `None` or the index is out of range.  Convenience for
    /// callers that need the standard "obstacle handle → plane" lookup
    /// before invoking [`Self::set_obstacle`-bearing setters].
    pub fn resolve_for_obstacle(
        obs: Option<ObstacleHandle>,
        obstacles: &[crate::sight_obstacle::SightObstacle],
    ) -> Option<Self> {
        obs.and_then(|h| {
            obstacles
                .get(usize::from(h.get()))
                .map(|o| Self::from_plane_points(&o.top_plane_points))
        })
    }

    pub fn from_plane_points(points: &[[f32; 3]; 3]) -> Self {
        let [p0, p1, p2] = *points;
        let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let v2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let nx = v1[1] * v2[2] - v1[2] * v2[1];
        let ny = v1[2] * v2[0] - v1[0] * v2[2];
        let nz = v1[0] * v2[1] - v1[1] * v2[0];
        if nz.abs() < 1e-9 {
            return Self {
                az: 0.0,
                bz: 0.0,
                dz: (p0[2] + p1[2] + p2[2]) / 3.0,
            };
        }
        Self {
            az: -nx / nz,
            bz: -ny / nz,
            dz: p0[2] + (nx * p0[0] + ny * p0[1]) / nz,
        }
    }
}

// ---------------------------------------------------------------------------
// Bitflag enums for computed state
// ---------------------------------------------------------------------------

bitflags! {
    /// Which increment vectors / direction have been computed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct IncrementComputed: u8 {
        const NONE      = 0;
        const MAP       = 1;
        const INCREMENT = 2;
        const DIRECTION = 4;
        const ALL       = 7;
    }
}

// ---------------------------------------------------------------------------
// Posture
// ---------------------------------------------------------------------------

/// Character posture.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Posture {
    Undefined = 0,
    #[default]
    Upright,
    Unused,
    Lying,
    OnLadder,
    OnWall,
    Siesta,
    Carried,
    Sitting,
    Flying,
    Crouched,
    CarryingCorpse,
    Dead,
    DeadBack,
    HelpingToClimb,
    CarryingOnShoulders,
    OnShoulders,
    StuckUnderNet,
    Tied,
    LeaningOut,
    SimulatingBeggar,
    Spy,
    Tree,
    AnonymousArcher,
    Leisure,
}

impl Posture {
    pub fn is_dead(self) -> bool {
        matches!(self, Self::Dead | Self::DeadBack)
    }

    pub fn is_lying(self) -> bool {
        matches!(
            self,
            Self::Lying | Self::DeadBack | Self::Dead | Self::Tied | Self::StuckUnderNet
        )
    }
}

// ---------------------------------------------------------------------------
// Opaque handles
// ---------------------------------------------------------------------------

/// Elevation-layer index.  Nominal newtype around `NonMaxU16` so
/// `Option<Layer>` gets niche-optimized to 2 bytes.  Layer indices are
/// small (typically 0..8); the on-disk sentinel is `0xFFFF` so a real
/// layer literally cannot hold it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct Layer(pub nonmax::NonMaxU16);

impl Layer {
    pub const ZERO: Layer = Layer(match nonmax::NonMaxU16::new(0) {
        Some(v) => v,
        None => unreachable!(),
    });
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl From<Layer> for u16 {
    #[inline]
    fn from(l: Layer) -> u16 {
        l.get()
    }
}
impl From<Layer> for u32 {
    #[inline]
    fn from(l: Layer) -> u32 {
        l.get() as u32
    }
}
impl From<Layer> for i16 {
    #[inline]
    fn from(l: Layer) -> i16 {
        l.get() as i16
    }
}
impl From<Layer> for usize {
    #[inline]
    fn from(l: Layer) -> usize {
        l.get() as usize
    }
}

impl Default for Layer {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

/// 16-sector compass direction (0..=15).  All compass arithmetic masks
/// with `& 15`; this newtype enforces the invariant and encapsulates
/// the rotation operations that appear scattered across the engine.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct Direction(u8);

impl Direction {
    pub const NORTH: Direction = Direction(0);
    pub const EAST: Direction = Direction(4);
    pub const SOUTH: Direction = Direction(8);
    pub const WEST: Direction = Direction(12);

    /// Construct from a raw compass value, masking to 0..=15.
    /// Accepts i32 so `from_raw(-1)` yields 15 (wrap-around) — matches
    /// the `(dir + X) & 15` idiom that pervades the engine.
    #[inline]
    pub fn from_raw(v: i32) -> Self {
        Self((v & 15) as u8)
    }

    /// Raw compass value as u8 (0..=15).
    #[inline]
    pub fn as_u8(self) -> u8 {
        self.0
    }

    /// Rotate by a signed delta, wrapping at 16.
    #[inline]
    pub fn rotate(self, delta: i32) -> Self {
        Self::from_raw(self.0 as i32 + delta)
    }

    /// Opposite direction (180°).
    #[inline]
    pub fn opposite(self) -> Self {
        self.rotate(8)
    }
}

impl From<Direction> for u8 {
    #[inline]
    fn from(d: Direction) -> u8 {
        d.0
    }
}
impl From<Direction> for u16 {
    #[inline]
    fn from(d: Direction) -> u16 {
        d.0 as u16
    }
}
impl From<Direction> for i16 {
    #[inline]
    fn from(d: Direction) -> i16 {
        d.0 as i16
    }
}
impl From<Direction> for u32 {
    #[inline]
    fn from(d: Direction) -> u32 {
        d.0 as u32
    }
}
impl From<Direction> for usize {
    #[inline]
    fn from(d: Direction) -> usize {
        d.0 as usize
    }
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Opaque handle to a sector.  Newtype around `NonMaxU16` (matches
/// the on-disk level-data sector index, where `0xFFFF` is reserved as the
/// binary-format "none" sentinel — see `crate::level_data`).  "No sector"
/// is represented by `Option<SectorHandle>::None`; the niche optimization
/// keeps `Option<SectorHandle>` the same size as `u16`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct SectorHandle(pub nonmax::NonMaxU16);

impl SectorHandle {
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl From<SectorHandle> for u16 {
    #[inline]
    fn from(h: SectorHandle) -> u16 {
        h.get()
    }
}

impl From<SectorHandle> for u32 {
    #[inline]
    fn from(h: SectorHandle) -> u32 {
        h.get() as u32
    }
}

impl From<SectorHandle> for i16 {
    #[inline]
    fn from(h: SectorHandle) -> i16 {
        h.get() as i16
    }
}

impl std::fmt::Display for SectorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

/// Opaque handle to a sight obstacle.  Newtype around `NonMaxU16`;
/// `0xFFFF` is reserved as the binary-format "none" sentinel so a real
/// handle literally cannot hold it.  "No obstacle" is represented by
/// `Option<ObstacleHandle>::None`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ObstacleHandle(pub nonmax::NonMaxU16);

impl ObstacleHandle {
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl From<ObstacleHandle> for u16 {
    #[inline]
    fn from(h: ObstacleHandle) -> u16 {
        h.get()
    }
}

impl From<ObstacleHandle> for usize {
    #[inline]
    fn from(h: ObstacleHandle) -> usize {
        h.get() as usize
    }
}

impl std::fmt::Display for ObstacleHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

/// Opaque handle to a door.
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
)]
pub struct DoorHandle(pub u32);
impl DoorHandle {
    pub const NULL: Self = Self(u32::MAX);
    pub fn is_null(self) -> bool {
        self == Self::NULL
    }
}

/// Opaque handle to an element.
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
)]
pub struct ElementHandle(pub u32);
impl ElementHandle {
    pub const NULL: Self = Self(u32::MAX);
    pub fn is_null(self) -> bool {
        self == Self::NULL
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default collision radius for a character.
pub const RADIUS_GUY: f32 = 4.0;

/// Inverse aspect ratio used for directional tolerance checks.
/// Value is `sec(55°) = 1 / cos(55°)`.
pub const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;

/// Isometric Y compression factor.  The reciprocal of
/// `INVERSE_ASPECT_RATIO`, = `cos(55°)` exactly — matches the
/// game's 55°-from-vertical camera tilt.  Used by box-shaped
/// pre-filters that build extents directly in raw map coordinates.
pub const ASPECT_RATIO: f32 = 1.0 / INVERSE_ASPECT_RATIO;

// ── Per-domain aspect ratios ─────────────
//
// Eugen Systems tuned several gameplay systems to use their own
// aspect-ratio constants.  The `SWORDFIGHT_*`, `MOVEMENT_*`, `FIREARMS`,
// and `MIRROR` variants have no live callers — the SWORDFIGHT one is
// referenced indirectly via the
// `engine::melee::INVERSE_SWORDFIGHT_ASPECT_RATIO` local alias, but
// the others are dead.  Only the two that get actually used are
// defined here; re-add the rest if a new caller appears.

/// Sword-fight aspect ratio — `1.0` in the shipping game.
/// The 0.5735 branch is commented out.  Keep the scaffolding at each
/// call site even though the multiplication is a no-op — if Eugen or
/// a mod flips it back to 0.5735, the callers pick up the change
/// automatically.
pub const SWORDFIGHT_ASPECT_RATIO: f32 = 1.0;

/// Inverse sword-fight aspect ratio — `1.0` in the shipping game.
/// The 1.7434 branch is commented out — Eugen disabled isometric
/// correction for sword combat, so
/// `StretchY(INVERSE_SWORDFIGHT_ASPECT_RATIO)` is a no-op.
pub const INVERSE_SWORDFIGHT_ASPECT_RATIO: f32 = 1.0;

/// Inverse aspect ratio for projectiles — `1.33` (thrown-object
/// range calculations).
pub const INVERSE_ASPECT_RATIO_PROJECTILES: f32 = 1.33;

// ---------------------------------------------------------------------------
// PositionInterface
// ---------------------------------------------------------------------------

/// Target-element context passed into [`PositionInterface::is_goal_reached`].
///
/// `is_goal_reached` reads the target's radius live when evaluating the
/// radius-slack arrival branch.  `PositionInterface` doesn't own a
/// reference to the target element, so the caller resolves it live and
/// passes it in.  `None` means "no target" and disables the slack
/// branch entirely.
#[derive(Debug, Clone, Copy)]
pub struct TargetInfo {
    pub radius: f32,
}

/// Position, movement, direction, and collision component for a game entity.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PositionInterface {
    // -- Computational state --
    computed_increment: IncrementComputed,

    // -- Positions --
    position: WorldPoint3D,
    position_map: MapPoint,

    old_position: WorldPoint3D,
    old_position_map: MapPoint,

    goal_map: MapPoint,
    goal_next_map: MapPoint,
    goal: WorldPoint3D,

    // -- Increments --
    increment: WorldVec3D,
    increment_map: MapVec,

    reversed_movement: bool,

    // -- Tolerance --
    tolerance: f32,
    directional_tolerance: bool,

    // -- Pathfinder indices --
    pathfinder_index: u16,
    pathfinder_index_alternate: u16,

    // -- Move boxes --
    move_box: BBox2D,
    move_box_alternate: BBox2D,

    use_emergency_lying_box: bool,

    move_box_map: MapBBox,

    // -- Direction --
    direction: Direction,
    direction_goal: Direction,
    slow_turn_count: u8,
    direction_count: i8,

    // -- Layer & sector --
    layer: Layer,
    sector: Option<SectorHandle>,
    layer_goal: Layer,
    sector_goal: Option<SectorHandle>,

    // -- Obstacle / plane --
    obstacle: Option<ObstacleHandle>,
    plane: Option<PlaneZCoeffs>,

    // -- Door --
    door: DoorHandle,
    door_direction: bool,

    // -- Material --
    material: crate::element::GameMaterial,

    // -- Anti-collision --
    goal_next_valid: bool,
    anti_collision_on: bool,
    pub deviated: bool,
    pub blocked_count: u16,
    pub box_blocked: MapBBox,
    pub radius: f32,
    pub radius_initial: f32,

    // -- Average speed --
    accumulate_movement_map: bool,
    accumulated_movement_map: MapVec,

    // -- Forecasted movement --
    forecasted_movement: WorldVec3D,
}

impl Default for PositionInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionInterface {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            computed_increment: IncrementComputed::NONE,

            position: WorldPoint3D::ZERO,
            position_map: MapPoint::ZERO,

            old_position: WorldPoint3D::ZERO,
            old_position_map: MapPoint::ZERO,

            goal_map: MapPoint::ZERO,
            goal_next_map: MapPoint::ZERO,
            goal: WorldPoint3D::ZERO,

            increment: WorldVec3D::ZERO,
            increment_map: MapVec::ZERO,

            reversed_movement: false,
            tolerance: 0.0,
            directional_tolerance: false,

            pathfinder_index: u16::MAX,
            pathfinder_index_alternate: u16::MAX,

            move_box: BBox2D::new(),
            move_box_alternate: BBox2D::new(),
            use_emergency_lying_box: false,
            move_box_map: MapBBox::new(),

            direction: Direction::NORTH,
            direction_goal: Direction::NORTH,
            slow_turn_count: 2,
            direction_count: 0,

            layer: Layer::ZERO,
            sector: None,
            layer_goal: Layer::ZERO,
            sector_goal: None,

            obstacle: None,
            plane: None,

            door: DoorHandle::NULL,
            door_direction: false,

            material: crate::element::GameMaterial::default(),

            goal_next_valid: false,
            anti_collision_on: true,
            deviated: false,
            blocked_count: 0,
            box_blocked: MapBBox::new(),
            radius: RADIUS_GUY,
            radius_initial: RADIUS_GUY,

            accumulate_movement_map: false,
            accumulated_movement_map: MapVec::ZERO,

            forecasted_movement: WorldVec3D::ZERO,
        }
    }

    // ====================================================================
    // Increment computed state
    // ====================================================================

    #[inline]
    pub fn is_increment_map_computed(&self) -> bool {
        self.computed_increment.contains(IncrementComputed::MAP)
    }
    #[inline]
    pub fn is_increment_3d_computed(&self) -> bool {
        self.computed_increment
            .contains(IncrementComputed::INCREMENT)
    }
    #[inline]
    pub fn is_increment_all_computed(&self) -> bool {
        self.computed_increment == IncrementComputed::ALL
    }

    #[inline]
    pub fn set_increment_map_computed(&mut self, v: bool) {
        if v {
            self.computed_increment |= IncrementComputed::MAP;
        } else {
            self.computed_increment -= IncrementComputed::MAP;
        }
    }
    #[inline]
    pub fn set_increment_3d_computed(&mut self, v: bool) {
        if v {
            self.computed_increment |= IncrementComputed::INCREMENT;
        } else {
            self.computed_increment -= IncrementComputed::INCREMENT;
        }
    }
    #[inline]
    pub fn reset_increment_computed(&mut self) {
        self.computed_increment = IncrementComputed::NONE;
    }

    // ====================================================================
    // Position getters / setters
    // ====================================================================

    #[inline]
    #[must_use = "method returns WorldPoint3D by value; `pi.get_position().x = v` silently modifies a temporary. Use `set_position` to mutate."]
    pub fn get_position(&self) -> WorldPoint3D {
        self.position
    }

    #[inline]
    #[must_use = "method returns MapPoint by value; use `set_map_position` to mutate."]
    pub fn map_position(&self) -> MapPoint {
        self.position_map
    }

    #[inline]
    #[must_use]
    pub fn get_elevation(&self) -> f32 {
        self.position.z
    }

    #[inline]
    pub fn set_position(&mut self, pt: WorldPoint3D) {
        self.position = pt;
        self.recompute_from_3d();
    }

    #[inline]
    pub fn set_map_position(&mut self, pt: MapPoint) {
        self.position_map = pt;
        self.recompute_from_map();
    }

    /// Assign the 2D map position without touching the 3D position or
    /// the derived sprite coordinates.  Only updates the map position
    /// and the map move-box — the 3D position is untouched because
    /// the two are sometimes mutated independently (e.g. action-point
    /// targets first lift Z via `set_position`, then overwrite the
    /// map to the action point so pathfinding seeks to the action
    /// point while rendering still happens at the elevated 3D point).
    /// Callers that want full re-derivation should use
    /// [`Self::set_map_position`] instead.
    #[inline]
    pub fn set_map_position_preserving_3d(&mut self, pt: MapPoint) {
        self.position_map = pt;
        self.move_box_map = self.get_move_box_offset(pt);
    }

    // Old position
    #[inline]
    pub fn old_map_position(&self) -> MapPoint {
        self.old_position_map
    }
    #[inline]
    pub fn set_old_position(&mut self, pt: WorldPoint3D) {
        self.old_position = pt;
    }
    #[inline]
    pub fn set_old_map_position(&mut self, pt: MapPoint) {
        self.old_position_map = pt;
    }

    #[inline]
    pub fn is_moving(&self) -> bool {
        self.position != self.old_position
    }
    #[inline]
    pub fn is_moving_map(&self) -> bool {
        self.position_map != self.old_position_map
    }

    // Goal
    #[inline]
    pub fn map_goal(&self) -> MapPoint {
        self.goal_map
    }
    #[inline]
    pub fn set_map_goal(&mut self, pt: MapPoint) {
        self.goal_map = pt;
        self.computed_increment = IncrementComputed::NONE;
    }
    #[inline]
    pub fn set_next_map_goal(&mut self, pt: MapPoint) {
        self.goal_next_map = pt;
        self.goal_next_valid = true;
    }
    #[inline]
    pub fn set_goal_next_valid(&mut self, v: bool) {
        self.goal_next_valid = v;
    }
    // Layer / sector
    #[inline]
    #[must_use]
    pub fn get_layer(&self) -> Layer {
        self.layer
    }
    #[inline]
    pub fn set_layer(&mut self, l: Layer) {
        self.layer = l;
    }
    #[inline]
    #[must_use]
    pub fn get_sector(&self) -> Option<SectorHandle> {
        self.sector
    }
    #[inline]
    pub fn set_sector(&mut self, s: Option<SectorHandle>) {
        self.sector = s;
    }
    // ====================================================================
    // Movement / increment
    // ====================================================================
    #[inline]
    pub fn get_increment(&self) -> WorldVec3D {
        assert!(self.is_increment_3d_computed());
        self.increment
    }

    #[inline]
    pub fn get_increment_map(&self) -> Vec2D {
        assert!(self.is_increment_map_computed());
        self.increment_map.to_geo()
    }

    #[inline]
    pub fn set_reversed_movement(&mut self, v: bool) {
        self.reversed_movement = v;
    }

    #[inline]
    pub fn set_increment_map(&mut self, v: Vec2D) {
        self.set_map_increment(MapVec::from_geo(v));
    }

    #[inline]
    pub fn set_map_increment(&mut self, v: MapVec) {
        self.computed_increment = IncrementComputed::MAP;
        self.increment_map = v;
    }

    /// Advance map position by `increment_map * distance`.
    pub fn update_position_map_scaled(&mut self, distance: f32) {
        assert!(self.is_increment_map_computed());
        let im = self.increment_map;
        self.position_map.x += im.x * distance;
        self.position_map.y += im.y * distance;
        self.recompute_from_map();
    }

    // ====================================================================
    // Direction (16-sector compass, 0 = north, CW)
    // ====================================================================

    #[inline]
    #[must_use]
    pub fn get_direction(&self) -> Direction {
        self.direction
    }
    #[inline]
    pub fn set_direction_instantly(&mut self, d: Direction) {
        self.direction = d;
        self.direction_goal = d;
    }
    #[inline]
    #[must_use]
    pub fn get_direction_goal(&self) -> Direction {
        self.direction_goal
    }
    #[inline]
    pub fn set_direction(&mut self, d: Direction) {
        self.direction_goal = d;
    }
    /// Turn one step toward the goal direction.  Returns `true` if still turning.
    pub fn turn(&mut self) -> bool {
        if self.deviated {
            return self.turn_anti_vibration();
        }

        let diff = (i32::from(self.direction_goal.as_u8()) - i32::from(self.direction.as_u8()))
            .rem_euclid(16);
        if diff == 0 {
            return false;
        }
        if diff < 8 {
            self.direction = self.direction.rotate(1);
        } else {
            self.direction = self.direction.rotate(-1);
        }
        true
    }

    /// Turn two steps toward the goal direction.
    pub fn turn_fast(&mut self) -> bool {
        let diff = (i32::from(self.direction_goal.as_u8()) - i32::from(self.direction.as_u8()))
            .rem_euclid(16);
        if diff == 0 {
            return false;
        }
        if diff < 8 {
            let step = if diff >= 2 { 2 } else { diff };
            self.direction = self.direction.rotate(step);
        } else {
            let step = if diff <= 14 { -2 } else { diff - 16 };
            self.direction = self.direction.rotate(step);
        }
        true
    }

    /// Slow turn (for horses). `slow_turn` controls the delay between steps.
    pub fn turn_slow(&mut self, slow_turn: u8) -> bool {
        if self.slow_turn_count == 0 {
            let diff = (i32::from(self.direction_goal.as_u8()) - i32::from(self.direction.as_u8()))
                .rem_euclid(16);
            if diff == 0 {
                return false;
            }
            self.slow_turn_count = slow_turn;
            if diff < 8 {
                self.direction = self.direction.rotate(1);
            } else {
                self.direction = self.direction.rotate(-1);
            }
            true
        } else {
            self.slow_turn_count -= 1;
            // Returns true while the slow-turn counter is draining,
            // even though no rotation happens this tick.
            true
        }
    }

    /// Very slow turn (delay of 5 ticks).
    pub fn turn_very_slow(&mut self) -> bool {
        self.turn_slow(5)
    }

    /// Anti-vibration turn: requires two consecutive same-direction requests
    /// before actually rotating.
    pub fn turn_anti_vibration(&mut self) -> bool {
        let diff = (i32::from(self.direction_goal.as_u8()) - i32::from(self.direction.as_u8()))
            .rem_euclid(16);
        if diff == 0 {
            return false;
        }
        if diff < 8 {
            if self.direction_count >= 2 {
                self.direction = self.direction.rotate(1);
            } else if self.direction_count < 0 {
                self.direction_count = 0;
            } else {
                self.direction_count += 1;
            }
        } else if self.direction_count <= -2 {
            self.direction = self.direction.rotate(-1);
        } else if self.direction_count > 0 {
            self.direction_count = 0;
        } else {
            self.direction_count -= 1;
        }
        true
    }

    #[inline]
    #[must_use]
    pub fn get_material(&self) -> crate::element::GameMaterial {
        self.material
    }
    #[inline]
    pub fn set_material(&mut self, m: crate::element::GameMaterial) {
        self.material = m;
    }

    #[inline]
    pub fn get_plane(&self) -> Option<&PlaneZCoeffs> {
        self.plane.as_ref()
    }
    #[inline]
    #[must_use]
    pub fn get_obstacle(&self) -> Option<ObstacleHandle> {
        self.obstacle
    }

    pub fn set_obstacle(&mut self, obs: Option<ObstacleHandle>, plane: Option<PlaneZCoeffs>) {
        // A non-null obstacle ALWAYS pairs with a non-null plane:
        // every sight obstacle owns a top plane, so callers must
        // pre-resolve the obstacle's top-plane coefficients before
        // calling. A `None` plane with `Some` obstacle silently dropped
        // elevation (no-plane leaf in `compute_position_3d`); the
        // assertion surfaces it instead.
        debug_assert!(
            obs.is_none() || plane.is_some(),
            "set_obstacle: when obstacle is Some, plane must also be Some \
             (sight obstacles always pair with their top plane)"
        );
        self.obstacle = obs;
        self.plane = plane;
        // Plane changed — refresh 3D position from current map position +
        // new plane so callers see a consistent 3D coordinate.  Sprite
        // and move-box depend on map only, so no further resync needed.
        self.position_3d_from_map();
        self.set_increment_3d_computed(false);
    }

    // ====================================================================
    // Move box
    // ====================================================================

    /// Current move box (centered on origin).
    #[inline]
    pub fn get_move_box(&self) -> &BBox2D {
        &self.move_box
    }

    /// Move box in map coordinates.
    pub fn get_move_box_map(&self) -> &MapBBox {
        &self.move_box_map
    }

    #[inline]
    pub fn set_move_box(&mut self, b: BBox2D) {
        self.move_box = b;
    }
    /// In-place equivalent of [`for_actor`] — applies the actor-specific
    /// pathfinder index, move box, and position to an existing PI.  Used
    /// when the PI is embedded (e.g. inside `Sprite`) and spawn code
    /// wants to configure it without constructing a new one.
    pub fn configure_for_actor(
        &mut self,
        pathfinder_idx: u8,
        half_diagonal: Option<crate::geo2d::Vec2D>,
        position_map: MapPoint,
    ) {
        use crate::geo2d;
        let hd = half_diagonal.unwrap_or(geo2d::pt(1.0, 1.0));
        self.set_pathfinder_index(pathfinder_idx as u16);
        self.set_move_box(BBox2D::from_corners(
            geo2d::pt(-hd.x, -hd.y),
            geo2d::pt(hd.x, hd.y),
        ));
        self.set_map_position(position_map);
    }

    /// Half diagonal of the current move box (bottom-right corner).
    #[must_use = "method returns Vec2D by value; `pi.get_half_diagonal().x = v` silently modifies a temporary."]
    pub fn get_half_diagonal(&self) -> Vec2D {
        self.move_box.bottom_right()
    }

    #[inline]
    pub fn get_pathfinder_index(&self) -> u16 {
        self.pathfinder_index
    }
    #[inline]
    pub fn set_pathfinder_index(&mut self, i: u16) {
        self.pathfinder_index = i;
    }
    #[inline]
    pub fn is_using_emergency_lying_box(&self) -> bool {
        self.use_emergency_lying_box
    }
    // Door
    #[inline]
    pub fn get_door(&self) -> DoorHandle {
        self.door
    }
    #[inline]
    pub fn get_door_direction(&self) -> bool {
        self.door_direction
    }

    // Tolerance
    #[inline]

    pub fn set_tolerance(&mut self, t: f32, directional: bool) {
        self.tolerance = t;
        self.directional_tolerance = directional;
    }

    // Forecasted movement
    #[inline]
    pub fn get_forecasted_movement(&self) -> WorldVec3D {
        self.forecasted_movement
    }

    pub fn reset_forecasted_movement(&mut self) {
        self.forecasted_movement = WorldVec3D::ZERO;
    }

    // ====================================================================
    // New move / displace
    // ====================================================================

    /// Snapshot current position as "old" before a new move step.
    pub fn new_move(&mut self) {
        self.old_position = self.position;
        self.old_position_map = self.position_map;
    }

    // ====================================================================
    // Internal eager re-sync helpers
    //
    // Every public position-mutating operation writes one authoritative field
    // and then calls the matching `recompute_from_*` so map/3D coordinates stay
    // in sync. C++ sprite position is reconstructed from Element sprite data at
    // call sites that need it.
    // ====================================================================

    /// Resync map + move_box_map from the current `position`.
    fn recompute_from_3d(&mut self) {
        let map = self.position.to_map();
        self.position_map = map;
        self.move_box_map = self.get_move_box_offset(map);
    }

    /// Resync 3D + move_box_map from the current `position_map`.
    fn recompute_from_map(&mut self) {
        let map = self.position_map;
        self.move_box_map = self.get_move_box_offset(map);
        self.position_3d_from_map();
    }

    /// Internal: reconstruct 3D from current `position_map` + plane.
    fn position_3d_from_map(&mut self) {
        let map = self.position_map;
        self.position.x = map.x;
        if let Some(p) = &self.plane {
            self.position.z = p.compute_z(map.x, map.y);
            self.position.y =
                crate::coordinates::GroundPoint::from_map_and_z(map, self.position.z).y;
        } else {
            self.position.y = map.y;
            self.position.z = 0.0;
        }
    }

    // ====================================================================
    // Compute increments
    // ====================================================================

    /// Derive map increment from 3D increment or from goal.
    pub fn compute_increment_map(&mut self) {
        if self.is_increment_map_computed() {
            return;
        }
        if self.is_increment_3d_computed() {
            let inc = self.increment;
            self.increment_map = MapVec::from_world_xyz(inc.x, inc.y, inc.z);
        } else {
            let map = self.position_map;
            let goal = self.goal_map;
            let v = geo2d::pt(goal.x - map.x, goal.y - map.y);
            self.increment_map = MapVec::from_geo(geo2d::normalize(v));
        }
        self.set_increment_map_computed(true);
    }

    /// Derive all increments + direction.
    pub fn compute_increment_all(&mut self, compute_direction: bool) {
        if self.is_increment_all_computed() {
            return;
        }

        let mut very_small = false;

        if self.is_increment_3d_computed() {
            let inc = self.increment;
            self.increment_map = MapVec::from_world_xyz(inc.x, inc.y, inc.z);
        } else if self.is_increment_map_computed() {
            let im = self.increment_map;
            self.increment.x = im.x;
            if let Some(p) = &self.plane {
                self.increment.z = p.compute_z_increment(im.x, im.y);
                self.increment.y =
                    crate::coordinates::GroundVec::from_map_and_z(im, self.increment.z).y;
            } else {
                self.increment.y = im.y;
                self.increment.z = 0.0;
            }
        } else {
            let map = self.position_map;
            let goal = self.goal_map;
            let mut v = geo2d::pt(goal.x - map.x, goal.y - map.y);

            very_small = v.x.abs().max(v.y.abs()) < 1.0;

            if v.x != 0.0 || v.y != 0.0 {
                v = geo2d::normalize(v);
                self.increment_map = MapVec::from_geo(v);

                self.increment.x = v.x;
                if let Some(p) = &self.plane {
                    self.increment.z = p.compute_z_increment(v.x, v.y);
                    self.increment.y = crate::coordinates::GroundVec::from_map_and_z(
                        MapVec::from_geo(v),
                        self.increment.z,
                    )
                    .y;
                } else {
                    self.increment.y = v.y;
                    self.increment.z = 0.0;
                }
            }
        }

        if compute_direction && !very_small {
            let dir = vector_to_direction(self.increment.x, self.increment.y);
            if self.reversed_movement {
                self.set_direction(dir.opposite());
            } else {
                self.set_direction(dir);
            }
        }

        self.computed_increment = IncrementComputed::ALL;
    }

    // ====================================================================
    // Goal reached
    // ====================================================================

    /// Check whether the entity has arrived at its goal.
    ///
    /// `grid` is a required parameter (no global singleton), and the
    /// target's radius comes from the caller-supplied `target` (read
    /// live).  Passing `None` for `target` disables the blocked-count
    /// radius-slack branch.
    pub fn is_goal_reached(&self, grid: &FastFindGrid, target: Option<TargetInfo>) -> bool {
        let map = self.position_map;
        let goal = self.goal_map;
        let im = self.increment_map;

        if self.deviated {
            if self.goal_next_valid {
                let hd = self.get_half_diagonal();
                grid.is_reachable_thick(map, self.goal_next_map, self.layer.get(), hd)
            } else if self.blocked_count == 0 {
                self.directional_goal_check(map, goal, im)
            } else {
                // The "close enough" radius factors in both actors'
                // collision radii so two bulky bodies can register as
                // arrived without their centers overlapping.  The
                // horse-mount shortcut (tight 10-unit threshold for
                // ridable animals) is omitted because no animals ship
                // in the game.
                let to_goal = goal - map;
                if let Some(t) = target {
                    let slack = self.radius + t.radius + 10.0;
                    if to_goal.x.abs().max(to_goal.y.abs()) < slack {
                        return true;
                    }
                }
                to_goal.x.abs().max(to_goal.y.abs()) < 10.0
            }
        } else {
            self.directional_goal_check(map, goal, im)
        }
    }

    fn directional_goal_check(&self, map: MapPoint, goal: MapPoint, im: MapVec) -> bool {
        let to_goal = goal - map;
        if !self.directional_tolerance {
            im.x * to_goal.x + im.y * to_goal.y <= self.tolerance
        } else {
            im.x * to_goal.x + im.y * to_goal.y * INVERSE_ASPECT_RATIO <= self.tolerance
        }
    }

    // ====================================================================
    // Average speed
    // ====================================================================

    #[inline]
    pub fn set_average_speed_needed(&mut self, v: bool) {
        self.accumulate_movement_map = v;
    }

    pub fn initialize_average_speed_map(&mut self, pt: MapPoint) {
        let map = self.map_position();
        self.accumulated_movement_map = MapVec::new(map.x - pt.x, map.y - pt.y);
    }

    pub fn get_average_speed_map(&mut self) -> MapVec {
        let avg = MapVec::new(
            self.accumulated_movement_map.x * 0.1,
            self.accumulated_movement_map.y * 0.1,
        );
        self.accumulated_movement_map.x -= avg.x;
        self.accumulated_movement_map.y -= avg.y;
        avg
    }

    // ====================================================================
    // Anti-collision
    // ====================================================================

    #[inline]
    pub fn is_anti_collision_on(&self) -> bool {
        self.anti_collision_on
    }
    pub fn set_anti_collision_on(&mut self, on: bool) {
        self.anti_collision_on = on;
        if !on {
            self.deviated = false;
            self.goal_next_valid = false;
        }
    }

    #[inline]
    pub fn is_deviated(&self) -> bool {
        self.deviated
    }
    #[inline]

    pub fn reset_box_blocked(&mut self) {
        self.box_blocked.reset();
        self.blocked_count = 0;
        self.radius = self.radius_initial;
    }

    /// Track whether the entity is stuck in a small area.
    pub fn update_box_blocked(&mut self, point: MapPoint) -> bool {
        if self.box_blocked.is_somewhere() && self.box_blocked.contains_point(point) {
            self.blocked_count += 1;
            if self.radius > 1.0 {
                self.radius -= 0.2;
            }
            false
        } else {
            let half = MapVec::new(0.49, 0.49);
            self.box_blocked.expand_point(point + half);
            self.box_blocked.expand_point(point - half);
            self.blocked_count = 0;
            self.radius = self.radius_initial;
            true
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked_count > 50
    }

    #[inline]
    pub fn get_radius(&self) -> f32 {
        self.radius
    }

    // ====================================================================
    // Actor-vs-actor anti-collision
    // ====================================================================

    /// Sort repulsive points and lines by projected distance to the
    /// actor's future position, filtering out those outside their
    /// `action_radius`.
    ///
    /// Thin wrapper over the free function [`sort_repulsive_objects`]
    /// that supplies the actor's origin and radius from `self`.
    pub fn sort_repulsive_objects(
        &self,
        pt_future: MapPoint,
        points: &mut Vec<(RepulsivePoint, f32)>,
        lines: &mut Vec<(RepulsiveLine, f32)>,
    ) {
        sort_repulsive_objects(self.position_map, pt_future, self.radius, points, lines);
    }
}

fn bubble_sort_ascending_by_f32<T>(v: &mut [(T, f32)]) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Bubble sort: multiple passes, early-exit when a pass had no swaps.
    for i in (1..n).rev() {
        let mut done = true;
        for j in 1..=i {
            if v[j].1 < v[j - 1].1 {
                v.swap(j - 1, j);
                done = false;
            }
        }
        if done {
            break;
        }
    }
}

/// Sort repulsive points and lines, factored into a free function so
/// callers outside `PositionInterface` (e.g. the engine's per-tick
/// movement step when not every repulsive input lives on `self`) can
/// re-use the same ordering.  Both lists are mutated in place:
/// far-away entries are removed and the survivors are bubble-sorted
/// by distance (nearest first).  The bubble sort is deliberate — the
/// algorithm relies on its deterministic tie-break behaviour for
/// replay stability.
pub fn sort_repulsive_objects(
    origin: MapPoint,
    pt_future: MapPoint,
    actor_radius: f32,
    points: &mut Vec<(RepulsivePoint, f32)>,
    lines: &mut Vec<(RepulsiveLine, f32)>,
) {
    let motion = pt_future - origin;
    let motion_unit = geo2d::normalize(motion.to_geo());
    // Direct normal (default direct=true): (-y, x).
    let motion_unit_normal = MapVec::new(-motion_unit.y, motion_unit.x);

    points.retain_mut(|(pt, dist)| {
        let rel = origin - pt.position;
        let projected = rel.x * motion_unit_normal.x + rel.y * motion_unit_normal.y;
        let d = projected - actor_radius - pt.radius;
        if d <= pt.action_radius {
            *dist = d;
            true
        } else {
            false
        }
    });

    lines.retain_mut(|(line, dist)| {
        let rel_origin = origin - line.a;
        let rel_future = pt_future - line.a;
        let d_origin = rel_origin.x * line.normal.x + rel_origin.y * line.normal.y
            - actor_radius
            - line.radius;
        let d_future = rel_future.x * line.normal.x + rel_future.y * line.normal.y
            - actor_radius
            - line.radius;
        let d = d_origin.min(d_future);
        if d <= line.action_radius {
            *dist = d;
            true
        } else {
            false
        }
    });

    bubble_sort_ascending_by_f32(points);
    bubble_sort_ascending_by_f32(lines);
}

/// Iteratively deviate the movement around repulsive points / lines.
///
/// Peel off the nearest object (point or line, whichever projects
/// closer), try to compute a deviation around it, re-sort, repeat
/// until the lists are empty.
///
/// Returns `(new_future_position, deviated)`.  `deviated == false`
/// means the straight move is fine; the caller should commit
/// `new_future_position` directly.
///
/// This is the pure-math portion of anti-collision — no state
/// mutation, no `FastFindGrid` calls, no blocked-count tracking.
/// Callers can use it for a simple "push apart without snagging on
/// motion lines" step, or wrap it with
/// [`PositionInterface::update_position_anti_collision`] for the full
/// semantics.
pub fn compute_deviated_future(
    origin: MapPoint,
    pt_future: MapPoint,
    actor_radius: f32,
    points: Vec<RepulsivePoint>,
    lines: Vec<RepulsiveLine>,
) -> (MapPoint, bool) {
    if points.is_empty() && lines.is_empty() {
        return (pt_future, false);
    }

    let mut points: Vec<(RepulsivePoint, f32)> = points.into_iter().map(|p| (p, 0.0)).collect();
    let mut lines: Vec<(RepulsiveLine, f32)> = lines.into_iter().map(|l| (l, 0.0)).collect();

    let origin_map = origin;
    let mut movement = pt_future - origin;
    if movement.x == 0.0 && movement.y == 0.0 {
        return (pt_future, false);
    }
    let mut future = pt_future;

    sort_repulsive_objects(origin, future, actor_radius, &mut points, &mut lines);

    let mut deviated = false;
    loop {
        while !points.is_empty() && (lines.is_empty() || points[0].1 <= lines[0].1) {
            let (pt, _d) = points.remove(0);
            let mag = movement.length();
            if let Some(dist_dest) = pt.is_deviating(future)
                && let Some(new_mov) =
                    pt.compute_deviation(movement, origin_map, mag, dist_dest, actor_radius)
            {
                movement = new_mov;
                future = origin_map + movement;
                deviated = true;
            }
            sort_repulsive_objects(origin, future, actor_radius, &mut points, &mut lines);
        }

        while !lines.is_empty() && (points.is_empty() || lines[0].1 < points[0].1) {
            let (line, _d) = lines.remove(0);
            let mag = movement.length();
            if let Some(dist_dest) = line.is_deviating(future)
                && let Some(new_mov) =
                    line.compute_deviation(movement, origin_map, mag, dist_dest, actor_radius)
            {
                movement = new_mov;
                future = origin_map + movement;
                deviated = true;
            }
            sort_repulsive_objects(origin, future, actor_radius, &mut points, &mut lines);
        }

        if points.is_empty() && lines.is_empty() {
            break;
        }
    }

    (future, deviated)
}

impl PositionInterface {
    // ====================================================================
    // Fast-find grid integration
    // ====================================================================

    /// Compute the grid cell `(cx, cy)` for the current map position.
    /// Uses the same 64-pixel cell size as `FastFindGrid`.
    pub fn grid_cell(&self) -> (u16, u16) {
        let map = self.position_map;
        let cx = (map.x as i32 / GRID_CELL_SIZE) as u16;
        let cy = (map.y as i32 / GRID_CELL_SIZE) as u16;
        (cx, cy)
    }

    /// Test whether the current map position is inside the grid bounds.
    pub fn is_inside_grid(&self, grid: &FastFindGrid) -> bool {
        grid.is_inside_grid_point(self.position_map)
    }

    /// Check whether the current map position (with its move box) is free of
    /// motion-line collisions on the current layer.
    pub fn is_position_authorized(&self, grid: &FastFindGrid) -> bool {
        let move_box_map = self.move_box_map.to_geo();
        let lines = grid.get_active_motion_line_indices(self.layer.get(), &move_box_map);
        for &line_idx in &lines {
            let line = &grid.level.lines[usize::from(line_idx)];
            if line.intersects_bbox(&move_box_map) {
                return false;
            }
        }
        true
    }

    // ====================================================================
    // Helpers
    // ====================================================================

    /// Offset the move box to a map position.
    fn get_move_box_offset(&self, pt: MapPoint) -> MapBBox {
        if self.move_box.is_somewhere() {
            MapBBox::from_corners(
                MapPoint::new(self.move_box.x_min() + pt.x, self.move_box.y_min() + pt.y),
                MapPoint::new(self.move_box.x_max() + pt.x, self.move_box.y_max() + pt.y),
            )
        } else {
            MapBBox::new()
        }
    }
}

// ---------------------------------------------------------------------------
// AnticollisionData — snapshot for save/restore
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AnticollisionData {
    pub map: MapPoint,
    pub increment_map: MapVec,
    pub deviated: bool,
    pub box_blocked: MapBBox,
    pub blocked_count: u16,
    pub radius: f32,
}

// ---------------------------------------------------------------------------
// Direction helper
// ---------------------------------------------------------------------------

/// Convert a 2D vector `(x, y)` to a 16-sector compass direction.
///
/// Sector 0 = north (negative Y), increasing clockwise.
pub fn vector_to_sector_0_to_15(x: f32, y: f32) -> i16 {
    if x == 0.0 && y == 0.0 {
        return 0;
    }
    // atan2 gives angle from positive X axis, counter-clockwise.
    // We want 0 = north (neg Y), clockwise.
    let angle = y.atan2(x); // radians, range (-π, π]
    // Rotate so 0 = north: subtract π/2, then negate for CW
    // sector = (angle + π/2) / (2π/16) = (angle + π/2) * 8/π
    let sector =
        ((angle + std::f32::consts::FRAC_PI_2) * 8.0 / std::f32::consts::PI).round() as i16;
    // Wrap to 0..15
    ((sector % 16) + 16) % 16
}

/// Convert a 2D vector `(x, y)` to a 16-sector [`Direction`].
///
/// Thin [`Direction`]-returning alias over [`vector_to_sector_0_to_15`]
/// for internal callers that want the strongly-typed compass value.
pub fn vector_to_direction(x: f32, y: f32) -> Direction {
    Direction::from_raw(vector_to_sector_0_to_15(x, y) as i32)
}

// ---------------------------------------------------------------------------
// Isometric-aware vector helpers
//
// Every gameplay call site needs to convert between map-space and the
// rotated viewing plane via `ASPECT_RATIO` (0.5735).  These helpers
// bake that convention in so callers don't each re-derive the sign of
// the Y-stretch.
// ---------------------------------------------------------------------------

/// Like [`vector_to_sector_0_to_15`] but takes a map-space vector and
/// applies the isometric Y-stretch before binning.
///
/// Equivalent to calling the bare helper on `(X, Y * INVERSE_ASPECT_RATIO)`.
/// Use this for any angular test on map coordinates (facing a target,
/// flight direction, etc).
#[inline]
pub fn vector_to_sector_0_to_15_iso(x: f32, y: f32) -> i16 {
    vector_to_sector_0_to_15(x, y * INVERSE_ASPECT_RATIO)
}

/// Isometric-space 2D vector squared-norm: `X² + (Y / ASPECT_RATIO)²`.
#[inline]
pub fn vector_square_norm_iso(x: f32, y: f32) -> f32 {
    let yi = y * INVERSE_ASPECT_RATIO;
    x * x + yi * yi
}

/// Isometric-space 2D vector norm: `sqrt(X² + (Y / ASPECT_RATIO)²)`.
#[inline]
pub fn vector_norm_iso(x: f32, y: f32) -> f32 {
    vector_square_norm_iso(x, y).sqrt()
}

/// Unit direction for a 16-sector compass value, compressed back into
/// isometric map space: `(tableX[idx], tableY[idx] * ASPECT_RATIO)`.
#[inline]
pub fn sector_to_vector_iso(sector: i16) -> [f32; 2] {
    let [x, y] = crate::shadow_polygon::sector_to_direction(sector);
    [x, y * ASPECT_RATIO]
}

/// Isometric normalize — scale `(x, y)` to unit length under
/// [`vector_norm_iso`].  Zero-length inputs return `(0, 0)`.
#[inline]
pub fn vector_normalize_iso(x: f32, y: f32) -> [f32; 2] {
    let n = vector_norm_iso(x, y);
    if n < f32::EPSILON {
        [0.0, 0.0]
    } else {
        [x / n, y / n]
    }
}

/// Isometric perpendicular — rotates the vector 90° with
/// aspect-correction: `direct = true` yields the left normal,
/// `false` the right.
#[inline]
pub fn vector_normal_iso(x: f32, y: f32, direct: bool) -> [f32; 2] {
    if direct {
        [-y * INVERSE_ASPECT_RATIO, x * ASPECT_RATIO]
    } else {
        [y * INVERSE_ASPECT_RATIO, -x * ASPECT_RATIO]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn p3(x: f32, y: f32, z: f32) -> WorldPoint3D {
        WorldPoint3D::new(x, y, z)
    }

    #[test]
    fn test_point3d_to_map() {
        let p = p3(10.0, 20.0, 5.0);
        let m = p.to_map();
        assert!((m.x - 10.0).abs() < 1e-6);
        assert!((m.y - 15.0).abs() < 1e-6);
    }

    #[test]
    fn test_typed_map_position_uses_projected_space() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(10.0, 20.0, 5.0));

        assert_eq!(pi.map_position(), MapPoint::new(10.0, 15.0));

        pi.set_map_position(MapPoint::new(30.0, 40.0));
        assert_eq!(pi.map_position(), MapPoint::new(30.0, 40.0));
        assert_eq!(pi.get_position(), p3(30.0, 40.0, 0.0));
    }

    fn d(v: i32) -> Direction {
        Direction::from_raw(v)
    }

    #[test]
    fn test_new_default() {
        let pi = PositionInterface::new();
        assert_eq!(pi.get_direction(), Direction::NORTH);
        assert!(pi.is_anti_collision_on());
        assert!(!pi.is_deviated());
        assert!((pi.get_radius() - RADIUS_GUY).abs() < 1e-6);
    }

    #[test]
    fn test_set_position_3d_eagerly_syncs_map() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(100.0, 200.0, 50.0));
        let map = pi.map_position();
        assert!((map.x - 100.0).abs() < 1e-4);
        assert!((map.y - 150.0).abs() < 1e-4); // y - z
    }

    #[test]
    fn test_set_position_3d_eagerly_syncs_map_and_move_box() {
        let mut pi = PositionInterface::new();
        pi.set_move_box(BBox2D::from_corners(
            geo2d::pt(0.0, 0.0),
            geo2d::pt(1.0, 1.0),
        ));
        pi.set_position(p3(100.0, 200.0, 0.0));

        let map = pi.map_position();
        assert!((map.x - 100.0).abs() < 1e-4);
        assert!((map.y - 200.0).abs() < 1e-4);

        let box_map = pi.get_move_box_map();
        assert!((box_map.x_min() - 100.0).abs() < 1e-4);
        assert!((box_map.y_min() - 200.0).abs() < 1e-4);
    }

    #[test]
    fn test_set_position_map_with_plane_eagerly_syncs_3d() {
        let mut pi = PositionInterface::new();
        pi.plane = Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.5,
            dz: 10.0,
        });
        pi.set_map_position(MapPoint::new(50.0, 100.0));

        let pos = pi.get_position();
        assert!((pos.x - 50.0).abs() < 1e-4);
        // z = (0.5 * 100 + 0 * 50 + 10) / (1 - 0.5) = 60/0.5 = 120
        assert!((pos.z - 120.0).abs() < 1e-3);
        assert!((pos.y - 220.0).abs() < 1e-3); // map.y + z
    }

    #[test]
    fn test_set_position_map_without_plane_zeroes_z() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(100.0, 200.0, 50.0));

        pi.set_map_position(MapPoint::new(75.0, 125.0));

        let pos = pi.get_position();
        assert!((pos.x - 75.0).abs() < 1e-4);
        assert!((pos.y - 125.0).abs() < 1e-4);
        assert!(pos.z.abs() < 1e-4);
    }

    #[test]
    fn test_plane_z_coeffs_from_flat_plane() {
        // Three coplanar points all at z = 5; the plane is flat.
        let pts = [[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [0.0, 10.0, 5.0]];
        let coeffs = PlaneZCoeffs::from_plane_points(&pts);
        assert!(coeffs.az.abs() < 1e-6);
        assert!(coeffs.bz.abs() < 1e-6);
        assert!((coeffs.dz - 5.0).abs() < 1e-6);
        // For a flat plane every map point yields z = 5.
        assert!((coeffs.compute_z(0.0, 0.0) - 5.0).abs() < 1e-4);
        assert!((coeffs.compute_z(123.0, -45.0) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_plane_z_coeffs_from_sloped_plane() {
        // A plane that rises +1 in z per +2 in world-y (slope 0.5 in y),
        // and is independent of x.  Three world-space points:
        //   (0, 0, 0), (10, 0, 0), (0, 10, 5)
        let pts = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 5.0]];
        let coeffs = PlaneZCoeffs::from_plane_points(&pts);
        assert!(coeffs.az.abs() < 1e-6);
        assert!((coeffs.bz - 0.5).abs() < 1e-6);
        assert!(coeffs.dz.abs() < 1e-6);
        // Iso projection: world_y = map_y + world_z.  At map (0, 0):
        //   z = (0.5 * 0 + 0 * 0 + 0) / (1 - 0.5) = 0
        assert!((coeffs.compute_z(0.0, 0.0)).abs() < 1e-4);
        // At map (5, 10) the plane resolves z = 0.5 * 10 / 0.5 = 10
        // (i.e. world_y = 20 → world_z = 10).
        assert!((coeffs.compute_z(5.0, 10.0) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn test_plane_z_coeffs_degenerate_collapses_to_average() {
        // Three collinear points → degenerate plane; fall back to mean z.
        let pts = [[0.0, 0.0, 1.0], [1.0, 0.0, 2.0], [2.0, 0.0, 3.0]];
        let coeffs = PlaneZCoeffs::from_plane_points(&pts);
        assert!(coeffs.az.abs() < 1e-6);
        assert!(coeffs.bz.abs() < 1e-6);
        assert!((coeffs.dz - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_turn_basic() {
        let mut pi = PositionInterface::new();
        pi.set_direction_instantly(d(0)); // north
        pi.set_direction(d(4)); // east

        // Should turn CW, one step per call
        for expected in 1..=4 {
            assert!(pi.turn());
            assert_eq!(pi.get_direction(), d(expected));
        }
        // Should stop at goal
        assert!(!pi.turn());
    }

    #[test]
    fn test_turn_ccw() {
        let mut pi = PositionInterface::new();
        pi.set_direction_instantly(d(2));
        pi.set_direction(d(14)); // 14 is -2 or CCW by 4 steps

        // Diff = (14-2) & 15 = 12, which is >= 8, so turn CCW
        assert!(pi.turn());
        assert_eq!(pi.get_direction(), d(1));
        assert!(pi.turn());
        assert_eq!(pi.get_direction(), d(0));
        assert!(pi.turn());
        assert_eq!(pi.get_direction(), d(15));
        assert!(pi.turn());
        assert_eq!(pi.get_direction(), d(14));
        assert!(!pi.turn());
    }

    #[test]
    fn test_turn_fast() {
        let mut pi = PositionInterface::new();
        pi.set_direction_instantly(d(0));
        pi.set_direction(d(6));

        assert!(pi.turn_fast());
        assert_eq!(pi.get_direction(), d(2));
        assert!(pi.turn_fast());
        assert_eq!(pi.get_direction(), d(4));
        assert!(pi.turn_fast());
        assert_eq!(pi.get_direction(), d(6));
        assert!(!pi.turn_fast());
    }

    #[test]
    fn test_turn_anti_vibration() {
        let mut pi = PositionInterface::new();
        pi.direction = d(0);
        pi.direction_goal = d(4);
        pi.direction_count = 0;

        // Needs 2 increments of direction_count before actually turning
        assert!(pi.turn_anti_vibration());
        assert_eq!(pi.direction, d(0)); // not yet
        assert_eq!(pi.direction_count, 1);

        assert!(pi.turn_anti_vibration());
        assert_eq!(pi.direction, d(0)); // count = 2 now
        assert_eq!(pi.direction_count, 2);

        assert!(pi.turn_anti_vibration());
        assert_eq!(pi.direction, d(1)); // now it turns
    }

    #[test]
    fn test_compute_increment_map_from_goal() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(0.0, 0.0));
        pi.set_map_goal(MapPoint::new(10.0, 0.0));

        pi.compute_increment_map();
        let im = pi.get_increment_map();
        assert!((im.x - 1.0).abs() < 1e-4);
        assert!(im.y.abs() < 1e-4);
    }

    #[test]
    fn test_compute_increment_all_with_plane() {
        let mut pi = PositionInterface::new();
        pi.plane = Some(PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        });
        pi.set_map_position(MapPoint::new(0.0, 0.0));
        pi.set_map_goal(MapPoint::new(0.0, 10.0));
        pi.compute_increment_all(true);

        assert!(pi.is_increment_all_computed());
        let inc = pi.get_increment();
        assert!(inc.x.abs() < 1e-4);
        assert!((inc.y - 1.0).abs() < 1e-4);
        assert!(inc.z.abs() < 1e-4);
    }

    #[test]
    fn test_is_goal_reached() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(50.0, 50.0));
        pi.set_map_goal(MapPoint::new(50.0, 50.0));
        pi.increment_map = MapVec::new(0.0, 1.0);
        pi.computed_increment = IncrementComputed::ALL;
        pi.tolerance = 0.0;

        let grid = FastFindGrid::new();
        assert!(pi.is_goal_reached(&grid, None));
    }

    #[test]
    fn test_is_goal_reached_behind() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(50.0, 51.0));
        pi.set_map_goal(MapPoint::new(50.0, 50.0));
        // Increment points in +Y direction, goal is behind us (dot < 0)
        pi.increment_map = MapVec::new(0.0, 1.0);
        pi.computed_increment = IncrementComputed::ALL;
        pi.tolerance = 0.0;

        let grid = FastFindGrid::new();
        assert!(pi.is_goal_reached(&grid, None)); // dot product is negative → ≤ 0
    }

    #[test]
    fn test_update_box_blocked() {
        let mut pi = PositionInterface::new();
        // First point: expands box, returns true
        assert!(pi.update_box_blocked(MapPoint::new(10.0, 10.0)));
        assert_eq!(pi.blocked_count, 0);

        // Same point: inside box, returns false (blocked)
        assert!(!pi.update_box_blocked(MapPoint::new(10.0, 10.0)));
        assert_eq!(pi.blocked_count, 1);

        // Far away point: expands box, returns true
        assert!(pi.update_box_blocked(MapPoint::new(100.0, 100.0)));
        assert_eq!(pi.blocked_count, 0);
    }

    #[test]
    fn test_posture_helpers() {
        assert!(Posture::Dead.is_dead());
        assert!(Posture::DeadBack.is_dead());
        assert!(!Posture::Upright.is_dead());

        assert!(Posture::Lying.is_lying());
        assert!(Posture::Tied.is_lying());
        assert!(!Posture::Crouched.is_lying());
    }

    #[test]
    fn test_average_speed() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(100.0, 200.0));
        pi.set_increment_map(geo2d::pt(1.0, 0.0));
        pi.set_average_speed_needed(true);
        pi.initialize_average_speed_map(MapPoint::new(90.0, 200.0));

        // Accumulated = (100-90, 0) = (10, 0)
        let avg = pi.get_average_speed_map();
        assert!((avg.x - 1.0).abs() < 1e-4); // 10 * 0.1
    }

    #[test]
    fn test_vector_to_sector() {
        // North (negative Y)
        assert_eq!(vector_to_sector_0_to_15(0.0, -1.0), 0);
        // East
        assert_eq!(vector_to_sector_0_to_15(1.0, 0.0), 4);
        // South
        assert_eq!(vector_to_sector_0_to_15(0.0, 1.0), 8);
        // West
        assert_eq!(vector_to_sector_0_to_15(-1.0, 0.0), 12);
    }

    #[test]
    fn test_iso_helpers_roundtrip() {
        // sector_to_vector_iso followed by vector_to_sector_0_to_15_iso
        // should recover the original sector.
        for sector in 0..16 {
            let [x, y] = sector_to_vector_iso(sector);
            assert_eq!(
                vector_to_sector_0_to_15_iso(x, y),
                sector,
                "sector {sector} did not round-trip",
            );
        }
    }

    #[test]
    fn test_vector_norm_iso() {
        // Pure-X component: norm == |X|
        assert!((vector_norm_iso(3.0, 0.0) - 3.0).abs() < 1e-4);
        // Pure-Y component: norm == |Y| * INVERSE_ASPECT_RATIO
        assert!((vector_norm_iso(0.0, 1.0) - INVERSE_ASPECT_RATIO).abs() < 1e-4);
    }

    #[test]
    fn test_vector_normal_iso_matches_original_aspect_scaling() {
        // direct=true: (-y / ASPECT_RATIO, x * ASPECT_RATIO)
        // direct=false flips the signs.
        let [lx, ly] = vector_normal_iso(10.0, 20.0, true);
        assert!((lx + 20.0 * INVERSE_ASPECT_RATIO).abs() < 1e-4);
        assert!((ly - 10.0 * ASPECT_RATIO).abs() < 1e-4);

        let [rx, ry] = vector_normal_iso(10.0, 20.0, false);
        assert!((rx - 20.0 * INVERSE_ASPECT_RATIO).abs() < 1e-4);
        assert!((ry + 10.0 * ASPECT_RATIO).abs() < 1e-4);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(10.0, 20.0, 5.0));
        pi.set_direction_instantly(d(7));

        let json = serde_json::to_string(&pi).unwrap();
        let pi2: PositionInterface = serde_json::from_str(&json).unwrap();
        assert_eq!(pi2.get_direction(), d(7));
        assert_eq!(pi2.position, p3(10.0, 20.0, 5.0));
    }

    #[test]
    fn test_new_move_and_is_moving() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(10.0, 20.0, 0.0));
        pi.new_move();
        assert!(!pi.is_moving());

        pi.set_position(p3(11.0, 20.0, 0.0));
        assert!(pi.is_moving());
    }

    // ── Grid integration tests ──

    #[test]
    fn test_grid_cell() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(200.0, 300.0));
        let (cx, cy) = pi.grid_cell();
        assert_eq!(cx, 3); // 200 / 64 = 3
        assert_eq!(cy, 4); // 300 / 64 = 4
    }

    #[test]
    fn test_is_inside_grid() {
        let mut grid = FastFindGrid::new();
        grid.size_map(10, 10); // 10*64 = 640 pixels wide/tall

        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(100.0, 100.0));
        assert!(pi.is_inside_grid(&grid));

        pi.set_map_position(MapPoint::new(700.0, 100.0));
        assert!(!pi.is_inside_grid(&grid));
    }

    #[test]
    fn test_is_position_authorized_empty_grid() {
        let mut grid = FastFindGrid::new();
        grid.size_map(10, 10);
        grid.allocate_layers(1);

        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(100.0, 100.0));
        // With no lines in the grid, any position is authorized
        assert!(pi.is_position_authorized(&grid));
    }
}
