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

use crate::coordinates::{
    MapBBox, MapPoint, MapVec, MoveBox, MoveBoxHalfDiagonal, WorldPoint3D, WorldVec3D,
};
use crate::fast_find_grid::{FastFindGrid, GRID_CELL_SIZE, SectorIndex};
use crate::geo2d;
use crate::repulsive::{RepulsiveLine, RepulsivePoint};

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

// ---------------------------------------------------------------------------
// Plane Z-coefficients
// ---------------------------------------------------------------------------

/// Z-computation coefficients: `z = az·x + bz·y + dz`.
///
/// The full plane lives on the sight obstacle; we cache only the
/// coefficients needed by the position math here.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PlaneZCoeffs {
    pub az: f32,
    pub bz: f32,
    pub dz: f32,
}

impl PlaneZCoeffs {
    /// Compute Z from world/ground-space `(x, y)`.
    ///
    /// Keep the multiplication and addition order identical to
    /// `SBGeoPlane3D::ComputeZ`; points lying exactly on a sloped plane can
    /// otherwise move by one ULP and wind up on the wrong side of it.
    #[inline]
    pub fn compute_world_z(&self, x: f32, y: f32) -> f32 {
        x * self.az + y * self.bz + self.dz
    }

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
                .get(h.get() as usize)
                .map(|o| Self::from_plane_points(&o.top_plane_points))
        })
    }

    pub fn from_plane_points(points: &[[f32; 3]; 3]) -> Self {
        let [p0, p1, p2] = *points;
        let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let v2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let mut nx = v1[1] * v2[2] - v2[1] * v1[2];
        let mut ny = v1[2] * v2[0] - v2[2] * v1[0];
        let mut nz = v1[0] * v2[1] - v2[0] * v1[1];
        if nz.abs() < 1e-9 {
            return Self {
                az: 0.0,
                bz: 0.0,
                dz: (p0[2] + p1[2] + p2[2]) / 3.0,
            };
        }

        // Preserve SBGeoPlane3D::InitializeAll's FLOAT operation sequence
        // rather than algebraically reducing the ratios. Native parity builds
        // force SSE2, giving these steps the same binary32 semantics as Rust.
        let norm = (nx * nx + ny * ny + nz * nz).sqrt();
        nx /= norm;
        ny /= norm;
        nz /= norm;
        let d = -p0[0] * nx - p0[1] * ny - p0[2] * nz;
        let k = -1.0 / nz;
        Self {
            az: nx * k,
            bz: ny * k,
            dz: d * k,
        }
    }
}

// ---------------------------------------------------------------------------
// Bitflag enums for computed state
// ---------------------------------------------------------------------------

bitflags! {
    /// Which serialized coordinate representations are currently valid.
    ///
    /// Rust normally keeps map and 3D coordinates eagerly synchronized, but
    /// Original v48 saves retain this lazy-computation mask independently.
    /// Keeping it here is required for an exact mid-motion restore.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub(crate) struct PositionComputed: u8 {
        const NONE   = 0;
        const THREE_D = 1;
        const MAP    = 2;
        const SPRITE = 4;
        const ALL    = 7;
    }

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

crate::bitcode_adapters::impl_native_bitcode_flags!(PositionComputed, u8);
crate::bitcode_adapters::impl_native_bitcode_flags!(IncrementComputed, u8);

impl robin_util::state_hash::StateHash for PositionComputed {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.bits(), state);
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
    bitcode::Encode,
    bitcode::Decode,
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

/// Elevation-layer index.
///
/// Original PositionInterface uses scalar `0xffff` as the special projectile
/// discriminator that selects a sight-obstacle reference instead of a
/// projection-area layer. Runtime `None` represents precisely that special
/// no-elevation-layer state, and legacy readers translate it only after they
/// have selected the correct pointer namespace. Layer zero remains live.
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
pub struct Layer(nonmax::NonMaxU16);

crate::bitcode_adapters::impl_native_bitcode_index!(Layer, u16);

impl Layer {
    pub const ZERO: Layer = Layer(nonmax::NonMaxU16::new(0).unwrap());
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

/// Index into the loaded pathfinder/move-box table. `0xffff` means
/// "unconfigured" in Original constructors and at binary boundaries.
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
pub struct PathfinderIndex(pub nonmax::NonMaxU16);

crate::bitcode_adapters::impl_native_bitcode_index!(PathfinderIndex, u16);

impl PathfinderIndex {
    #[inline]
    pub fn new(value: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(value).map(Self)
    }

    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl From<PathfinderIndex> for u16 {
    #[inline]
    fn from(index: PathfinderIndex) -> Self {
        index.get()
    }
}

impl From<PathfinderIndex> for usize {
    #[inline]
    fn from(index: PathfinderIndex) -> Self {
        usize::from(index.get())
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
    bitcode::Encode,
    bitcode::Decode,
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

/// Reference to a sector's public number, optionally enriched with its exact
/// live arena identity. Public number `-1` is representable because Original
/// uses it for the real out-of-map sector; nullable serialized pointers must
/// translate their `0xffff` marker to `None` at the binary boundary.
#[derive(Debug, Clone, Copy, robin_state_hash_derive::StateHash)]
pub struct SectorHandle {
    public: crate::sector::SectorNumber,
    /// Optional exact live arena object. Equality and hashing deliberately
    /// remain public-number based; Original pointer comparisons must opt in
    /// through [`Self::arena_index`]. Keeping the companion in this copyable
    /// value lets `RHposition +/- RHvector`-style copies retain provenance
    /// without changing every `Position` literal.
    arena: Option<crate::fast_find_grid::SectorIndex>,
}

impl crate::bitcode_adapters::NativeBitcode for SectorHandle {
    type Wire = (i16, Option<crate::fast_find_grid::SectorIndex>);

    fn to_wire(&self) -> Self::Wire {
        (self.public.get(), self.arena)
    }

    fn from_wire((public, arena): Self::Wire) -> Self {
        Self {
            public: crate::sector::SectorNumber::new(public),
            arena,
        }
    }
}

crate::bitcode_adapters::impl_native_bitcode!(SectorHandle);

impl SectorHandle {
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        Some(Self {
            public: crate::sector::SectorNumber::new(v as i16),
            arena: None,
        })
    }

    #[inline]
    pub fn from_number(public: crate::sector::SectorNumber) -> Self {
        Self {
            public,
            arena: None,
        }
    }

    /// Decode a nullable serialized `RHSector*` slot. This is intentionally
    /// distinct from [`Self::new`], because level/authored sector number
    /// `0xffff` denotes the real out-of-map sector.
    #[inline]
    pub fn from_serialized_pointer(v: u16) -> Option<Self> {
        (v != u16::MAX).then(|| Self::from_number(crate::sector::SectorNumber::new(v as i16)))
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.public.get() as u16
    }

    #[inline]
    pub fn number(self) -> crate::sector::SectorNumber {
        self.public
    }

    #[inline]
    pub fn with_arena_index(self, arena: crate::fast_find_grid::SectorIndex) -> Self {
        Self {
            public: self.public,
            arena: Some(arena),
        }
    }

    #[inline]
    pub fn arena_index(self) -> Option<crate::fast_find_grid::SectorIndex> {
        self.arena
    }
}

impl PartialEq for SectorHandle {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.public == other.public
    }
}

impl Eq for SectorHandle {}

impl std::hash::Hash for SectorHandle {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.public.hash(state);
    }
}

#[derive(Serialize, Deserialize)]
struct SectorHandleSerde {
    number: crate::sector::SectorNumber,
    arena: Option<crate::fast_find_grid::SectorIndex>,
}

impl Serialize for SectorHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            SectorHandleSerde {
                number: self.public,
                arena: self.arena,
            }
            .serialize(serializer)
        } else {
            (self.public.get(), self.arena).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for SectorHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let SectorHandleSerde { number, arena } = SectorHandleSerde::deserialize(deserializer)?;
            Ok(Self {
                public: number,
                arena,
            })
        } else {
            let (public, arena) =
                <(i16, Option<crate::fast_find_grid::SectorIndex>)>::deserialize(deserializer)?;
            Ok(Self {
                public: crate::sector::SectorNumber::new(public),
                arena,
            })
        }
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

/// Compatibility name for the canonical flat static+dynamic sight-obstacle
/// index. The former `u16` wrapper narrowed valid raycast results; all live
/// surfaces now use the full `SightObstacleIndex` identity.
pub use crate::sight_obstacle::SightObstacleIndex as ObstacleHandle;

/// Compatibility name for the canonical door-table index. Runtime absence
/// is represented by `Option<DoorIndex>`; there is no second door-handle ID
/// space in the position component.
pub use crate::gate::DoorIndex as DoorHandle;

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
    bitcode::Encode,
    bitcode::Decode,
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
pub const ASPECT_RATIO: f32 = 0.573_576_45;

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

/// Match `SBGeoVector2D::Normalize`'s source operation order under the
/// schema-5 scalar-SSE floating-point contract.
fn normalize_map_vector_original(v: MapVec) -> Option<MapVec> {
    let norm = (v.x * v.x + v.y * v.y).sqrt();
    (norm != 0.0).then_some(MapVec {
        x: v.x / norm,
        y: v.y / norm,
    })
}

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
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PositionInterface {
    // -- Serialized lazy-computation state --
    computed_position: PositionComputed,

    // -- Computational state --
    computed_increment: IncrementComputed,

    // -- Positions --
    position: WorldPoint3D,
    position_map: MapPoint,
    position_sprite: MapPoint,
    #[serde(default)]
    position_sprite_valid: bool,

    old_position: WorldPoint3D,
    old_position_map: MapPoint,
    old_position_sprite: MapPoint,

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
    #[serde(deserialize_with = "deserialize_required_option")]
    pathfinder_index: Option<PathfinderIndex>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pathfinder_index_alternate: Option<PathfinderIndex>,

    // -- Move boxes --
    move_box: MoveBox,
    move_box_alternate: MoveBox,

    use_emergency_lying_box: bool,

    move_box_map: MapBBox,

    // -- Direction --
    direction: Direction,
    direction_goal: Direction,
    slow_turn_count: u8,
    direction_count: i8,

    // Original keeps posture in RHPositionInterface. ElementData remains
    // Rust's gameplay-facing owner, but retaining both serialized values here
    // prevents old-posture state from being lost during save adoption.
    saved_posture: crate::element::Posture,
    saved_old_posture: crate::element::Posture,

    // -- Layer & sector --
    #[serde(deserialize_with = "deserialize_required_option")]
    layer: Option<Layer>,
    #[serde(deserialize_with = "deserialize_required_option")]
    sector: Option<SectorHandle>,
    /// Exact `FastFindGrid::level.sectors` arena identity paired with
    /// `sector`.  `SectorHandle` retains the compact/public sector number,
    /// which is not sufficient to distinguish overlapping sector objects.
    #[serde(deserialize_with = "deserialize_required_option")]
    sector_index: Option<SectorIndex>,
    #[serde(deserialize_with = "deserialize_required_option")]
    layer_goal: Option<Layer>,
    #[serde(deserialize_with = "deserialize_required_option")]
    sector_goal: Option<SectorHandle>,
    /// Arena identity paired with `sector_goal`.
    #[serde(deserialize_with = "deserialize_required_option")]
    sector_goal_index: Option<SectorIndex>,

    // -- Obstacle / plane --
    #[serde(deserialize_with = "deserialize_required_option")]
    obstacle: Option<ObstacleHandle>,
    #[serde(deserialize_with = "deserialize_required_option")]
    plane: Option<PlaneZCoeffs>,

    // -- Door --
    #[serde(deserialize_with = "deserialize_required_option")]
    door: Option<DoorHandle>,
    door_direction: bool,

    // -- Material --
    // Original serializes the enum as an unchecked ULONG. Projectile
    // trajectory points can copy the sentinel 9 or uninitialized raw storage
    // here, so save adoption must retain all bits until gameplay actually
    // consumes the material.
    material: u32,

    // -- Anti-collision --
    goal_next_valid: bool,
    anti_collision_on: bool,
    pub deviated: bool,
    pub blocked_count: u16,
    pub box_blocked: MapBBox,
    pub radius: f32,
    pub radius_initial: f32,
    #[serde(deserialize_with = "deserialize_required_option")]
    saved_target_element: Option<crate::entity_id::EntityId>,

    // -- Average speed --
    accumulate_movement_map: bool,
    accumulated_movement_map: MapVec,

    // -- Forecasted movement --
    forecasted_movement: WorldVec3D,
}

/// Fully validated state written by Original v48
/// `RHPositionInterface::Serialize`.
///
/// Fields omitted here are explicitly not serialized by the Original:
/// pathfinder indices, centered move boxes, sprite center, and initial
/// radius. [`PositionInterface::restore_v48_serialized_state`] preserves
/// those mission-initialized values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PositionInterfaceV48State {
    pub computed_position: PositionComputed,
    pub computed_increment: IncrementComputed,
    /// Raw unchecked `RHmaterial` storage. Validated only by live material
    /// access, matching Original's plain `CHECKENUM` byte copy.
    pub material: u32,
    pub posture: crate::element::Posture,
    pub old_posture: crate::element::Posture,
    pub direction: Direction,
    pub direction_goal: Direction,
    pub slow_turn_count: u8,
    pub layer: Option<Layer>,
    pub layer_goal: Option<Layer>,
    pub tolerance: f32,
    pub directional_tolerance: bool,
    pub accumulate_movement_map: bool,
    pub anti_collision_on: bool,
    pub goal_next_valid: bool,
    pub deviated: bool,
    pub direction_count: i8,
    pub door_direction: bool,
    pub reversed_movement: bool,
    pub blocked_count: u16,
    pub radius: f32,
    pub use_emergency_lying_box: bool,
    pub sector: Option<SectorHandle>,
    pub sector_index: Option<SectorIndex>,
    pub sector_goal: Option<SectorHandle>,
    pub sector_goal_index: Option<SectorIndex>,
    pub door: Option<DoorHandle>,
    pub obstacle: Option<ObstacleHandle>,
    pub plane: Option<PlaneZCoeffs>,
    pub target_element: Option<crate::entity_id::EntityId>,
    pub position: WorldPoint3D,
    pub map: MapPoint,
    pub sprite: MapPoint,
    pub old_position: WorldPoint3D,
    pub old_map: MapPoint,
    pub old_sprite: MapPoint,
    pub goal_map: MapPoint,
    pub goal_next_map: MapPoint,
    pub goal: WorldPoint3D,
    pub increment: WorldVec3D,
    pub increment_map: MapVec,
    pub accumulated_movement_map: MapVec,
    pub forecasted_movement: WorldVec3D,
    pub move_box_map: MapBBox,
    pub blocked_box: MapBBox,
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
            computed_position: PositionComputed::ALL,
            computed_increment: IncrementComputed::NONE,

            position: WorldPoint3D::ZERO,
            position_map: MapPoint::ZERO,
            position_sprite: MapPoint::ZERO,
            position_sprite_valid: false,

            old_position: WorldPoint3D::ZERO,
            old_position_map: MapPoint::ZERO,
            old_position_sprite: MapPoint::ZERO,

            goal_map: MapPoint::ZERO,
            goal_next_map: MapPoint::ZERO,
            goal: WorldPoint3D::ZERO,

            increment: WorldVec3D::ZERO,
            increment_map: MapVec::ZERO,

            reversed_movement: false,
            tolerance: 0.0,
            directional_tolerance: false,

            pathfinder_index: None,
            pathfinder_index_alternate: None,

            move_box: MoveBox::new(),
            move_box_alternate: MoveBox::new(),
            use_emergency_lying_box: false,
            move_box_map: MapBBox::new(),

            direction: Direction::NORTH,
            direction_goal: Direction::NORTH,
            slow_turn_count: 2,
            direction_count: 0,
            saved_posture: crate::element::Posture::Undefined,
            saved_old_posture: crate::element::Posture::Undefined,

            layer: Some(Layer::ZERO),
            sector: None,
            sector_index: None,
            layer_goal: Some(Layer::ZERO),
            sector_goal: None,
            sector_goal_index: None,

            obstacle: None,
            plane: None,

            door: None,
            door_direction: false,

            material: crate::element::GameMaterial::default().as_u32(),

            goal_next_valid: false,
            anti_collision_on: true,
            deviated: false,
            blocked_count: 0,
            box_blocked: MapBBox::new(),
            radius: RADIUS_GUY,
            radius_initial: RADIUS_GUY,
            saved_target_element: None,

            accumulate_movement_map: false,
            accumulated_movement_map: MapVec::ZERO,

            forecasted_movement: WorldVec3D::ZERO,
        }
    }

    /// Atomically install one preflighted Original v48 serialized state.
    ///
    /// No setters are used: their eager recomputation would overwrite the
    /// independently serialized map/3D/sprite caches and computation masks.
    pub(crate) fn restore_v48_serialized_state(&mut self, state: PositionInterfaceV48State) {
        self.computed_position = state.computed_position;
        self.computed_increment = state.computed_increment;
        self.material = state.material;
        self.saved_posture = state.posture;
        self.saved_old_posture = state.old_posture;
        self.direction = state.direction;
        self.direction_goal = state.direction_goal;
        self.slow_turn_count = state.slow_turn_count;
        self.layer = state.layer;
        self.layer_goal = state.layer_goal;
        self.tolerance = state.tolerance;
        self.directional_tolerance = state.directional_tolerance;
        self.accumulate_movement_map = state.accumulate_movement_map;
        self.anti_collision_on = state.anti_collision_on;
        self.goal_next_valid = state.goal_next_valid;
        self.deviated = state.deviated;
        self.direction_count = state.direction_count;
        self.door_direction = state.door_direction;
        self.reversed_movement = state.reversed_movement;
        self.blocked_count = state.blocked_count;
        self.radius = state.radius;
        self.use_emergency_lying_box = state.use_emergency_lying_box;
        self.set_sector_topology(state.sector, state.sector_index);
        self.set_goal_sector_topology(state.sector_goal, state.sector_goal_index);
        self.door = state.door;
        self.obstacle = state.obstacle;
        self.plane = state.plane;
        self.saved_target_element = state.target_element;
        self.position = state.position;
        self.position_map = state.map;
        self.position_sprite = state.sprite;
        self.position_sprite_valid = state.computed_position.contains(PositionComputed::SPRITE);
        self.old_position = state.old_position;
        self.old_position_map = state.old_map;
        self.old_position_sprite = state.old_sprite;
        self.goal_map = state.goal_map;
        self.goal_next_map = state.goal_next_map;
        self.goal = state.goal;
        self.increment = state.increment;
        self.increment_map = state.increment_map;
        self.accumulated_movement_map = state.accumulated_movement_map;
        self.forecasted_movement = state.forecasted_movement;
        self.move_box_map = state.move_box_map;
        self.box_blocked = state.blocked_box;
    }

    pub(crate) fn v48_serialized_state(&self) -> PositionInterfaceV48State {
        PositionInterfaceV48State {
            computed_position: self.computed_position,
            computed_increment: self.computed_increment,
            material: self.material,
            posture: self.saved_posture,
            old_posture: self.saved_old_posture,
            direction: self.direction,
            direction_goal: self.direction_goal,
            slow_turn_count: self.slow_turn_count,
            layer: self.layer,
            layer_goal: self.layer_goal,
            tolerance: self.tolerance,
            directional_tolerance: self.directional_tolerance,
            accumulate_movement_map: self.accumulate_movement_map,
            anti_collision_on: self.anti_collision_on,
            goal_next_valid: self.goal_next_valid,
            deviated: self.deviated,
            direction_count: self.direction_count,
            door_direction: self.door_direction,
            reversed_movement: self.reversed_movement,
            blocked_count: self.blocked_count,
            radius: self.radius,
            use_emergency_lying_box: self.use_emergency_lying_box,
            sector: self.sector,
            sector_index: self.sector_index,
            sector_goal: self.sector_goal,
            sector_goal_index: self.sector_goal_index,
            door: self.door,
            obstacle: self.obstacle,
            plane: self.plane,
            target_element: self.saved_target_element,
            position: self.position,
            map: self.position_map,
            sprite: self.position_sprite,
            old_position: self.old_position,
            old_map: self.old_position_map,
            old_sprite: self.old_position_sprite,
            goal_map: self.goal_map,
            goal_next_map: self.goal_next_map,
            goal: self.goal,
            increment: self.increment,
            increment_map: self.increment_map,
            accumulated_movement_map: self.accumulated_movement_map,
            forecasted_movement: self.forecasted_movement,
            move_box_map: self.move_box_map,
            blocked_box: self.box_blocked,
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

    /// Return the exact cached C++ `mposPositionSprite` when one has been
    /// established by mission placement or restored from a legacy save.
    #[inline]
    pub(crate) fn cached_sprite_position(&self) -> Option<MapPoint> {
        self.position_sprite_valid.then_some(self.position_sprite)
    }

    /// Install the exact integer sprite-space top-left used by C++ gameplay
    /// hotspot queries.
    #[inline]
    pub(crate) fn set_cached_sprite_position(&mut self, position: MapPoint) {
        self.position_sprite = position;
        self.position_sprite_valid = true;
    }

    #[inline]
    #[must_use]
    pub fn get_elevation(&self) -> f32 {
        self.position.z
    }

    #[inline]
    pub fn set_position(&mut self, pt: WorldPoint3D) {
        self.position = pt;
        self.position_sprite_valid = false;
        self.recompute_from_3d();
        // Original `RHPositionInterface::SetPosition(SBGeoPoint3D)` makes
        // the supplied world point the sole authoritative projection.  In
        // particular, `PerformFlight` can install a 3-D point while no
        // ground obstacle/plane is attached; a same-slot ComputeEyesPoint
        // must then read that flight point instead of resolving map space
        // back onto z=0.
        self.computed_position = PositionComputed::THREE_D;
    }

    /// Restore the cached 3D coordinate while declaring every position
    /// projection valid, matching Original's `SetPositionAllComputed()`.
    ///
    /// This compatibility operation deliberately does not derive map or
    /// sprite coordinates from `pt`. Original uses it after queuing an
    /// outdoor corpse drop to preserve the current-frame projections until
    /// the delayed map position is applied by the next Actor Hourglass.
    #[inline]
    pub(crate) fn restore_cached_position_all_computed(&mut self, pt: WorldPoint3D) {
        self.position = pt;
        self.computed_position = PositionComputed::ALL;
    }

    #[inline]
    pub fn set_map_position(&mut self, pt: MapPoint) {
        self.position_map = pt;
        self.position_sprite_valid = false;
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
    pub fn old_position(&self) -> WorldPoint3D {
        self.old_position
    }
    #[inline]
    pub fn old_elevation(&self) -> f32 {
        self.old_position.z
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
    pub fn layer_goal(&self) -> Layer {
        self.layer_goal
            .expect("position has no goal layer; legacy no-layer state escaped its boundary")
    }
    #[inline]
    pub fn optional_layer_goal(&self) -> Option<Layer> {
        self.layer_goal
    }
    #[inline]
    pub fn set_layer_goal(&mut self, layer: Layer) {
        self.layer_goal = Some(layer);
    }
    #[inline]
    pub(crate) fn clear_layer_goal(&mut self) {
        self.layer_goal = None;
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
            .expect("position has no layer; legacy no-layer state escaped its boundary")
    }
    #[inline]
    #[must_use]
    pub fn optional_layer(&self) -> Option<Layer> {
        self.layer
    }
    #[inline]
    pub fn set_layer(&mut self, l: Layer) {
        self.layer = Some(l);
    }
    #[inline]
    pub(crate) fn clear_layer(&mut self) {
        self.layer = None;
    }
    #[inline]
    #[must_use]
    pub fn get_sector(&self) -> Option<SectorHandle> {
        self.sector
    }
    /// Return the public sector number and its exact arena identity as one
    /// topology snapshot.
    #[inline]
    #[must_use]
    pub fn get_sector_topology(&self) -> (Option<SectorHandle>, Option<SectorIndex>) {
        (self.sector, self.sector_index)
    }
    /// Atomically replace the current sector number and arena identity.
    #[inline]
    pub fn set_sector_topology(
        &mut self,
        sector: Option<SectorHandle>,
        sector_index: Option<SectorIndex>,
    ) {
        assert!(
            sector.is_some() || sector_index.is_none(),
            "a sector arena identity requires a public sector handle"
        );
        self.sector = sector.map(|handle| match sector_index {
            Some(index) => handle.with_arena_index(index),
            None => SectorHandle::new(handle.get()).expect("live sector handle became null"),
        });
        self.sector_index = sector_index;
    }
    #[inline]
    pub fn set_sector(&mut self, s: Option<SectorHandle>) {
        // A number-only write has no proof that the prior arena object still
        // applies. Clear it rather than retaining stale pointer provenance.
        self.set_sector_topology(s, None);
    }
    /// Return the goal sector number and exact arena identity together.
    #[inline]
    #[must_use]
    pub fn get_goal_sector_topology(&self) -> (Option<SectorHandle>, Option<SectorIndex>) {
        (self.sector_goal, self.sector_goal_index)
    }
    /// Atomically replace the goal sector number and arena identity.
    #[inline]
    pub fn set_goal_sector_topology(
        &mut self,
        sector: Option<SectorHandle>,
        sector_index: Option<SectorIndex>,
    ) {
        assert!(
            sector.is_some() || sector_index.is_none(),
            "a goal-sector arena identity requires a public sector handle"
        );
        self.sector_goal = sector.map(|handle| match sector_index {
            Some(index) => handle.with_arena_index(index),
            None => SectorHandle::new(handle.get()).expect("goal sector handle became null"),
        });
        self.sector_goal_index = sector_index;
    }
    /// Number-only goal-sector writes deliberately discard arena provenance.
    #[inline]
    pub fn set_goal_sector(&mut self, sector: Option<SectorHandle>) {
        self.set_goal_sector_topology(sector, None);
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
    pub fn get_increment_map(&self) -> MapVec {
        assert!(self.is_increment_map_computed());
        self.increment_map
    }

    /// Return the stored map increment without asserting that this
    /// representation is currently authoritative.
    ///
    /// This is for complete state snapshots and legacy serialization only.
    /// Gameplay must use [`Self::get_increment_map`], whose validity assertion
    /// prevents stale storage from being mistaken for the live increment.
    #[inline]
    pub fn raw_increment_map(&self) -> MapVec {
        self.increment_map
    }

    #[inline]
    pub fn set_reversed_movement(&mut self, v: bool) {
        self.reversed_movement = v;
    }

    #[inline]
    pub fn set_map_increment(&mut self, v: MapVec) {
        self.computed_increment = IncrementComputed::MAP;
        self.increment_map = v;
    }

    /// Stop an arrived motion while preserving the fact that every increment
    /// representation is valid. This matches `SetIncrementMap(0)`,
    /// `SetIncrement(0)`, then `SetIncrementAllComputed()` in the Original.
    pub fn zero_all_increments(&mut self) {
        self.increment_map = MapVec::ZERO;
        self.increment = WorldVec3D::ZERO;
        self.computed_increment = IncrementComputed::ALL;
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

    /// Hidden direction latches exposed only to opt-in parity diagnostics.
    /// Gameplay must continue to mutate these exclusively through the
    /// ordinary `Turn*` and movement paths.
    #[inline]
    pub(crate) fn parity_turn_provenance_state(&self) -> (bool, i8, u8, u8) {
        (
            self.deviated,
            self.direction_count,
            self.direction.as_u8(),
            self.direction_goal.as_u8(),
        )
    }

    /// Drop the anti-collision deviation latch when a zero-distance
    /// (already-at-destination) transition order initializes.
    ///
    /// The available C++ source never clears `mbDeviated` on this path, yet
    /// the shipped Linux game observably does: on Savegame_010 replay-014
    /// (batch-19 direction cluster), Soldier 61 ends a deviated walk with a
    /// +2 anti-vibration count (frames 972-983 show the full standing
    /// hysteresis, so the latch is still set), then a seek whose destination
    /// equals his current position initializes its start transition on frame
    /// 1014.  After that, frame 1030's in-place `Turning` rotates
    /// counter-clockwise on its very first `Turn()` call and frame 1032's
    /// `RaisingShield` turn rotates clockwise on its first call.  No
    /// count-changing call happens between the two, so no `msbDirectionCount`
    /// value can satisfy both anti-vibration gates (`>= +2` and `<= -2`);
    /// the original must be taking the plain non-deviated `Turn()` path,
    /// which means the latch itself was dropped.  The same trace bounds the
    /// clear tightly: mid-walk waypoint order inits (frame 168, Soldier 63)
    /// and a start-transition init with a real destination (frame 204,
    /// Soldier 70) both keep the deviated anti-collision recovery branch, so
    /// only the aligned in-place initialization may clear.
    ///
    /// This replaces the earlier "prime the count for the current goal"
    /// model (task #545), which reproduced the clockwise cases but could
    /// never reproduce a first-call counter-clockwise rotation from a
    /// +2 count.  A plain `Turn()` rotates immediately in both directions,
    /// matching every observation, and #545's aligned-transition case is
    /// equally satisfied by the hysteresis-free path.
    /// `msbDirectionCount` is left untouched, mirroring how
    /// `SetAntiCollisionOn(false)` clears only the latch, never the counter.
    pub fn clear_deviated_for_aligned_transition_start(&mut self) {
        self.deviated = false;
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
        crate::element::GameMaterial::from_u32(self.material)
    }
    #[inline]
    pub fn set_material(&mut self, m: crate::element::GameMaterial) {
        self.material = m.as_u32();
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
    pub fn get_move_box(&self) -> &MoveBox {
        &self.move_box
    }

    /// Move box in map coordinates.
    pub fn get_move_box_map(&self) -> &MapBBox {
        &self.move_box_map
    }

    #[inline]
    pub fn set_move_box(&mut self, b: MoveBox) {
        self.move_box = b;
    }
    /// In-place equivalent of [`for_actor`] — applies the actor-specific
    /// pathfinder index, move box, and position to an existing PI.  Used
    /// when the PI is embedded (e.g. inside `Sprite`) and spawn code
    /// wants to configure it without constructing a new one.
    pub fn configure_for_actor(
        &mut self,
        pathfinder_idx: PathfinderIndex,
        half_diagonal: MoveBoxHalfDiagonal,
        position_map: MapPoint,
    ) {
        let hd = half_diagonal;
        self.set_pathfinder_index(pathfinder_idx);
        self.set_move_box(MoveBox::from_corners(
            MapVec::new(-hd.x, -hd.y),
            MapVec::new(hd.x, hd.y),
        ));
        self.set_map_position(position_map);
    }

    /// Half diagonal of the current move box (bottom-right corner).
    #[must_use = "method returns MoveBoxHalfDiagonal by value; `pi.get_half_diagonal().x = v` silently modifies a temporary."]
    pub fn get_half_diagonal(&self) -> MoveBoxHalfDiagonal {
        let hd = self.move_box.bottom_right();
        MoveBoxHalfDiagonal::new(hd.x, hd.y)
    }

    #[inline]
    pub fn get_pathfinder_index(&self) -> Option<PathfinderIndex> {
        self.pathfinder_index
    }
    #[inline]
    pub fn set_pathfinder_index(&mut self, index: PathfinderIndex) {
        self.pathfinder_index = Some(index);
    }
    #[inline]
    pub fn clear_pathfinder_index(&mut self) {
        self.pathfinder_index = None;
    }
    #[inline]
    pub fn is_using_emergency_lying_box(&self) -> bool {
        self.use_emergency_lying_box
    }
    // Door
    #[inline]
    pub fn get_door(&self) -> Option<DoorHandle> {
        self.door
    }
    #[inline]
    pub(crate) fn set_door(&mut self, door: DoorHandle, direction: bool) {
        self.door = Some(door);
        self.door_direction = direction;
    }
    #[inline]
    pub(crate) fn clear_door(&mut self) {
        self.door = None;
    }
    #[cfg(test)]
    pub(crate) fn set_door_for_test(&mut self, door: DoorHandle) {
        self.door = Some(door);
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

    #[inline]
    #[must_use]
    pub fn get_tolerance(&self) -> f32 {
        self.tolerance
    }

    // Forecasted movement
    #[inline]
    pub fn get_forecasted_movement(&self) -> WorldVec3D {
        self.forecasted_movement
    }

    pub fn reset_forecasted_movement(&mut self) {
        self.forecasted_movement = WorldVec3D::ZERO;
    }

    /// Refresh the per-frame movement forecast after a committed motion
    /// step.  `distance` is the effective distance the step travelled
    /// (speed factor and turn slowdown already folded in) and
    /// `wait_time` is the sprite's wait time for the frame reached by
    /// the step, plus one.
    ///
    /// Consumers (arrow / stone / apple leading) read this to aim ahead
    /// of a walking victim, so it has to be refreshed on every motion
    /// commit rather than derived on demand: the increment is cleared
    /// once the actor arrives, but the last forecast lingers until the
    /// next animation change resets it.
    pub fn update_forecasted_movement(&mut self, distance: f32, wait_time: u16) {
        let inc = self.increment;
        let wait = f32::from(wait_time);
        self.forecasted_movement = WorldVec3D {
            x: (distance * inc.x) / wait,
            y: (distance * inc.y) / wait,
            z: (distance * inc.z) / wait,
        };
    }

    // ====================================================================
    // New move / displace
    // ====================================================================

    /// Snapshot current position as "old" before a new move step.
    pub fn new_move(&mut self) {
        self.old_position = self.position;
        self.old_position_map = self.position_map;
    }

    /// Mark the current location as a settled/non-moving position.
    ///
    /// Special motions such as jumps drive position outside
    /// `PerformMotion`'s normal new-order setup. Once they finish, stale
    /// movement goals from the pre-special-motion walk must not make
    /// `is_moving_map` / `is_in_motion` report phantom movement.
    pub fn settle_current_position(&mut self) {
        self.old_position = self.position;
        self.old_position_map = self.position_map;
        self.goal_map = self.position_map;
        self.goal_next_valid = false;
        self.increment = WorldVec3D::ZERO;
        self.increment_map = MapVec::ZERO;
        self.computed_increment = IncrementComputed::NONE;
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
            let v = goal - map;
            self.increment_map = normalize_map_vector_original(v).unwrap_or(MapVec::ZERO);
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
            let v = goal - map;
            // Original stores the subtraction before checking whether it is
            // zero. This clears a stale map direction at an exact-position
            // goal while intentionally leaving the 3D increment untouched.
            self.increment_map = v;

            very_small = v.x.abs().max(v.y.abs()) < 1.0;

            if v.x != 0.0 || v.y != 0.0 {
                let v = normalize_map_vector_original(v)
                    .expect("nonzero movement vector must have a finite positive norm");
                self.increment_map = v;

                self.increment.x = v.x;
                if let Some(p) = &self.plane {
                    self.increment.z = p.compute_z_increment(v.x, v.y);
                    self.increment.y =
                        crate::coordinates::GroundVec::from_map_and_z(v, self.increment.z).y;
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
                grid.is_reachable_thick(map, self.goal_next_map, self.get_layer().get(), hd)
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

    /// Goal test for motion whose anti-collision is disabled, and which can
    /// therefore never be deviated. Equivalent to [`Self::is_goal_reached`]
    /// without needing a grid: only the deviated branch consults one.
    #[must_use]
    pub fn is_goal_reached_undeviated(&self) -> bool {
        debug_assert!(
            !self.deviated,
            "is_goal_reached_undeviated called on a deviated position interface"
        );
        self.directional_goal_check(self.position_map, self.goal_map, self.increment_map)
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

    /// Element exempted from this actor's repulsive-neighbour scan.
    /// Original stores this pointer directly on `RHPositionInterface` and
    /// refreshes it only when `RHSprite::PerformMotion` initializes a new
    /// order; it therefore survives sequence interruption and cleanup.
    #[inline]
    pub fn target_element(&self) -> Option<crate::entity_id::EntityId> {
        self.saved_target_element
    }

    #[inline]
    pub fn set_target_element(&mut self, target: Option<crate::entity_id::EntityId>) {
        self.saved_target_element = target;
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
    movement_distance: f32,
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
            if let Some(dist_dest) = pt.is_deviating(future)
                && let Some(new_mov) = pt.compute_deviation(
                    movement,
                    origin_map,
                    movement_distance,
                    dist_dest,
                    actor_radius,
                )
            {
                movement = new_mov;
                future = origin_map + movement;
                deviated = true;
            }
            sort_repulsive_objects(origin, future, actor_radius, &mut points, &mut lines);
        }

        while !lines.is_empty() && (points.is_empty() || lines[0].1 < points[0].1) {
            let (line, _d) = lines.remove(0);
            if let Some(dist_dest) = line.is_deviating(future)
                && let Some(new_mov) = line.compute_deviation(
                    movement,
                    origin_map,
                    movement_distance,
                    dist_dest,
                    actor_radius,
                )
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
        let move_box_map = self.move_box_map;
        let lines = grid.get_active_motion_line_indices(self.get_layer().get(), &move_box_map);
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
            self.move_box.translated(pt)
        } else {
            MapBBox::new()
        }
    }
}

// ---------------------------------------------------------------------------
// AnticollisionData — snapshot for save/restore
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    vector_to_sector_0_to_15_with_aspect(x, y, 1.0)
}

pub(crate) fn vector_to_sector_0_to_15_with_aspect(x: f32, y: f32, aspect_ratio: f32) -> i16 {
    if x == 0.0 && y == 0.0 {
        return 0;
    }
    // Preserve `SBGeoVector2D::GetSector0to15`'s literal half-plane
    // classifier. An atan/round implementation is mathematically similar but
    // disagrees on f32 boundary vectors produced by anti-collision.
    const SIN_PI_SIXTEENTH: f32 = 0.195_090_32;
    const COS_PI_SIXTEENTH: f32 = 0.980_785_25;
    const TAN_PI_EIGHTH: f32 = 0.414_213_57;

    // Preserve Original's left-associative f32 operation order. Scaling Y by
    // the reciprocal before calling the aspect-1 classifier is algebraically
    // equivalent but can choose the other sector for exact boundary vectors.
    let mut rotated_x = x * COS_PI_SIXTEENTH * aspect_ratio - y * SIN_PI_SIXTEENTH;
    let mut rotated_y = x * SIN_PI_SIXTEENTH * aspect_ratio + y * COS_PI_SIXTEENTH;
    let west = rotated_x < 0.0;
    if west {
        rotated_x = -rotated_x;
    }
    let south = rotated_y > 0.0;
    if !south {
        rotated_y = -rotated_y;
    }
    let east_west_quarter = rotated_y < rotated_x;
    let skew_eighth = if east_west_quarter {
        rotated_y > rotated_x * TAN_PI_EIGHTH
    } else {
        rotated_x > rotated_y * TAN_PI_EIGHTH
    };

    ((u8::from(west) << 3)
        | (u8::from(west ^ south) << 2)
        | (u8::from(west ^ south ^ east_west_quarter) << 1)
        | u8::from(west ^ south ^ east_west_quarter ^ skew_eighth)) as i16
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
    vector_to_sector_0_to_15_with_aspect(x, y, ASPECT_RATIO)
}

/// Isometric-space 2D vector squared-norm: `X² + (Y / ASPECT_RATIO)²`.
#[inline]
pub fn vector_square_norm_iso(x: f32, y: f32) -> f32 {
    // SBGeoVector2D::SquareNorm/Norm divides by the supplied aspect ratio.
    // Multiplying by the precomputed reciprocal is algebraically equivalent
    // but not bit-equivalent in binary32, and the one-ULP difference feeds
    // directly into AI retreat/combat destinations.
    let yi = y / ASPECT_RATIO;
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
    // `SBGeoVector2D::GetNormal` divides by the aspect ratio here.  Using
    // `INVERSE_ASPECT_RATIO` is algebraically equivalent but can differ by
    // one ULP in binary32, which changes authoritative AI destinations.
    if direct {
        [-y / ASPECT_RATIO, x * ASPECT_RATIO]
    } else {
        [y / ASPECT_RATIO, -x * ASPECT_RATIO]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_handle_roundtrips_current_signed_exact_schema() {
        let number_only = SectorHandle::new(23).unwrap();
        let exact =
            number_only.with_arena_index(crate::fast_find_grid::SectorIndex::new(41).unwrap());
        let out_of_map = SectorHandle::from_number(crate::sector::SectorNumber::new(-1));

        assert_eq!(
            serde_json::to_string(&number_only).unwrap(),
            r#"{"number":23,"arena":null}"#
        );
        assert_eq!(
            serde_json::to_string(&exact).unwrap(),
            r#"{"number":23,"arena":41}"#
        );
        assert_eq!(
            serde_json::to_string(&out_of_map).unwrap(),
            r#"{"number":-1,"arena":null}"#
        );
        let number_only_from_json: SectorHandle =
            serde_json::from_str(r#"{"number":23,"arena":null}"#).unwrap();
        let exact_from_json: SectorHandle =
            serde_json::from_str(r#"{"number":23,"arena":41}"#).unwrap();
        assert_eq!(number_only_from_json.number(), number_only.number());
        assert_eq!(number_only_from_json.arena_index(), None);
        assert_eq!(exact_from_json.get(), exact.get());
        assert_eq!(exact_from_json.arena_index(), exact.arena_index());
        assert!(serde_json::from_str::<SectorHandle>("23").is_err());
        assert!(serde_json::from_str::<SectorHandle>(r#"{"public":23,"arena":41}"#).is_err());
        assert!(serde_json::from_str::<SectorHandle>(r#"{"number":23}"#).is_err());

        let values = vec![number_only, exact, out_of_map];
        let bytes = bitcode::encode(&values);
        let restored: Vec<SectorHandle> =
            bitcode::decode(&bytes).expect("decode mixed sector handles");
        assert_eq!(restored.len(), values.len());
        for (restored, value) in restored.into_iter().zip(values) {
            assert_eq!(restored.get(), value.get());
            assert_eq!(restored.arena_index(), value.arena_index());
        }
    }

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
        pi.set_obstacle(None, None);
        pi.set_position(p3(100.0, 200.0, 50.0));
        let map = pi.map_position();
        assert!((map.x - 100.0).abs() < 1e-4);
        assert!((map.y - 150.0).abs() < 1e-4); // y - z
        assert_eq!(pi.get_position(), p3(100.0, 200.0, 50.0));
        assert_eq!(pi.computed_position, PositionComputed::THREE_D);
    }

    #[test]
    fn test_set_position_3d_eagerly_syncs_map_and_move_box() {
        let mut pi = PositionInterface::new();
        pi.set_move_box(MoveBox::from_corners(
            MapVec::new(0.0, 0.0),
            MapVec::new(1.0, 1.0),
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
    fn plane_coefficients_and_projection_match_original_sse_rounding() {
        let pts = [
            [
                f32::from_bits(0x440c_a539),
                f32::from_bits(0x44fd_fbe4),
                f32::from_bits(0x4316_0042),
            ],
            [
                f32::from_bits(0x42db_66b4),
                f32::from_bits(0x44ef_68a9),
                f32::from_bits(0x3a83_126f),
            ],
            [
                f32::from_bits(0x441a_c40e),
                f32::from_bits(0x44f4_f65d),
                f32::from_bits(0x4316_0042),
            ],
        ];

        let coeffs = PlaneZCoeffs::from_plane_points(&pts);
        assert_eq!(coeffs.az.to_bits(), 0x3e8d_246a);
        assert_eq!(coeffs.bz.to_bits(), 0x3e5c_e9d4);
        assert_eq!(coeffs.dz.to_bits(), 0xc3dd_b755);
        assert_eq!(
            coeffs
                .compute_z(f32::from_bits(0x4320_0706), f32::from_bits(0x44ea_06e8))
                .to_bits(),
            0x40bb_1f7f
        );
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
    fn in_place_transition_drops_deviation_latch_for_pending_turn() {
        let mut pi = PositionInterface::new();
        pi.direction = d(12);
        pi.direction_goal = d(10);
        pi.deviated = true;
        pi.direction_count = 0;

        pi.clear_deviated_for_aligned_transition_start();

        assert!(pi.turn());
        assert_eq!(pi.direction, d(11));
    }

    #[test]
    fn aligned_in_place_transition_makes_later_goal_turn_immediate() {
        let mut pi = PositionInterface::new();
        pi.direction = d(6);
        pi.direction_goal = d(6);
        pi.deviated = true;
        pi.direction_count = 2;

        pi.clear_deviated_for_aligned_transition_start();
        assert_eq!(pi.direction_count, 2, "only the latch drops, not the count");

        pi.direction_goal = d(9);
        assert!(pi.turn());
        assert_eq!(pi.direction, d(7));
    }

    /// Savegame_010 replay-014 frame 1030 (Soldier 61): after an aligned
    /// in-place transition start, a *counter-clockwise* goal must also
    /// rotate on the very first `Turn()` call even though the retained
    /// anti-vibration count is +2.  While the latch is set this is
    /// impossible — `TurnAntiVibration` needs `count <= -2` for the first
    /// counter-clockwise step — which is what proves the shipped game drops
    /// the latch rather than priming the count.
    #[test]
    fn aligned_in_place_transition_makes_ccw_turn_immediate_despite_cw_count() {
        let mut pi = PositionInterface::new();
        pi.direction = d(15);
        pi.direction_goal = d(15);
        pi.deviated = true;
        pi.direction_count = 2;

        // Latch still set: the counter-clockwise request is absorbed by the
        // anti-vibration counter instead of rotating.
        pi.direction_goal = d(14);
        assert!(pi.turn());
        assert_eq!(pi.direction, d(15), "anti-vibration holds while latched");
        assert_eq!(pi.direction_count, 0);

        // With the aligned-transition clear, the same request rotates
        // immediately, and a subsequent clockwise request does too
        // (frame 1032's RaisingShield turn).
        pi.deviated = true;
        pi.direction_count = 2;
        pi.clear_deviated_for_aligned_transition_start();
        assert!(pi.turn());
        assert_eq!(pi.direction, d(14));
        pi.direction_goal = d(15);
        assert!(pi.turn());
        assert_eq!(pi.direction, d(15));
        assert!(!pi.turn(), "aligned direction reports finished");
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
    fn increment_normalization_divides_like_original_vector() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(1834.0, 1771.0));
        pi.set_map_goal(MapPoint::new(1_838.447_6, 1_766.972_8));

        pi.compute_increment_all(false);

        let increment = pi.get_increment_map();
        assert_eq!(increment.x.to_bits(), 0x3f3dc409);
        assert_eq!(increment.y.to_bits(), 0xbf2bd408);
    }

    #[test]
    fn zero_goal_overwrites_map_increment_but_preserves_3d_increment() {
        let mut pi = PositionInterface::new();
        pi.set_map_position(MapPoint::new(50.0, 75.0));
        pi.increment_map = MapVec::new(-0.4, -0.9);
        pi.increment = WorldVec3D::new(0.25, 0.5, 0.75);
        pi.computed_increment = IncrementComputed::NONE;
        pi.set_map_goal(MapPoint::new(50.0, 75.0));

        pi.compute_increment_all(false);

        assert_eq!(pi.get_increment_map(), MapVec::ZERO);
        assert_eq!(pi.get_increment(), WorldVec3D::new(0.25, 0.5, 0.75));
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
        pi.set_map_increment(MapVec::new(1.0, 0.0));
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
    fn test_iso_sector_matches_original_for_shallow_frame_1121_vector() {
        let dx = 63.314_94_f32;
        let dy = 7.342_773_4_f32;

        assert_eq!(vector_to_sector_0_to_15(dx, dy), 4);
        assert_eq!(vector_to_sector_0_to_15_iso(dx, dy), 5);
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
    fn current_serde_rejects_legacy_spatial_sentinels() {
        let mut encoded = serde_json::to_value(PositionInterface::new()).unwrap();
        let object = encoded.as_object_mut().unwrap();
        object.insert("pathfinder_index".into(), serde_json::json!(u16::MAX));
        object.insert("pathfinder_index_alternate".into(), serde_json::json!(7));
        object.insert("layer".into(), serde_json::json!(u16::MAX));
        object.insert("layer_goal".into(), serde_json::json!(3));
        object.insert("door".into(), serde_json::json!(u32::MAX));

        assert!(serde_json::from_value::<PositionInterface>(encoded).is_err());
    }

    #[test]
    fn native_bitcode_roundtrips_typed_spatial_absence() {
        let mut pi = PositionInterface::new();
        pi.clear_layer();
        pi.clear_layer_goal();
        pi.clear_pathfinder_index();
        pi.clear_door();

        let encoded = bitcode::encode(&pi);
        let restored: PositionInterface = bitcode::decode(&encoded).unwrap();
        assert_eq!(restored.optional_layer(), None);
        assert_eq!(restored.optional_layer_goal(), None);
        assert_eq!(restored.get_pathfinder_index(), None);
        assert_eq!(restored.get_door(), None);
    }

    #[test]
    fn sector_topology_is_atomic_and_number_only_writes_clear_identity() {
        let mut pi = PositionInterface::new();
        let sector = SectorHandle::new(18);
        let sector_index = SectorIndex::new(41);
        let goal = SectorHandle::new(19);
        let goal_index = SectorIndex::new(57);

        pi.set_sector_topology(sector, sector_index);
        pi.set_goal_sector_topology(goal, goal_index);
        assert_eq!(pi.get_sector_topology(), (sector, sector_index));
        assert_eq!(pi.get_goal_sector_topology(), (goal, goal_index));

        pi.set_sector(sector);
        pi.set_goal_sector(goal);
        assert_eq!(pi.get_sector_topology(), (sector, None));
        assert_eq!(pi.get_goal_sector_topology(), (goal, None));
    }

    #[test]
    fn sector_indices_roundtrip_and_are_required_by_current_serde() {
        let mut pi = PositionInterface::new();
        pi.set_sector_topology(SectorHandle::new(18), SectorIndex::new(41));
        pi.set_goal_sector_topology(SectorHandle::new(19), SectorIndex::new(57));

        let encoded = serde_json::to_value(&pi).unwrap();
        let restored: PositionInterface = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(restored.get_sector_topology(), pi.get_sector_topology());
        assert_eq!(
            restored.get_goal_sector_topology(),
            pi.get_goal_sector_topology()
        );

        let mut legacy = encoded;
        let object = legacy.as_object_mut().unwrap();
        object.remove("sector_index");
        object.remove("sector_goal_index");
        assert!(serde_json::from_value::<PositionInterface>(legacy).is_err());
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

    #[test]
    fn settle_current_position_clears_stale_motion_state() {
        let mut pi = PositionInterface::new();
        pi.set_position(p3(10.0, 20.0, 0.0));
        pi.set_old_position(p3(-10.0, -20.0, 0.0));
        pi.set_old_map_position(MapPoint::new(-10.0, -20.0));
        pi.set_map_goal(MapPoint::new(30.0, 40.0));
        pi.set_next_map_goal(MapPoint::new(50.0, 60.0));
        pi.compute_increment_all(true);

        assert!(pi.is_moving_map());
        assert_ne!(pi.map_goal(), pi.map_position());

        pi.settle_current_position();

        assert!(!pi.is_moving());
        assert!(!pi.is_moving_map());
        assert_eq!(pi.map_goal(), pi.map_position());
        assert!(!pi.is_increment_map_computed());
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
