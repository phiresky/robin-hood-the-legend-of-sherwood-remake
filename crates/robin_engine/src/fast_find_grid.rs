//! Spatial acceleration grid for the game world.
//!
//! The grid divides the map into 64×64 pixel cells. Each cell stores
//! indices of game objects (lines, sectors, obstacles, etc.) that overlap it.
//! This allows efficient spatial queries for pathfinding and collision detection.
//!
//! This module focuses on the pathfinding-relevant subset:
//! - Grid structure and cell-based lookups
//! - Motion line storage and querying
//! - Thick movement corridor reachability checks
//! - Position authorization (no obstacle intersection)
//!
//! Scope: patches, sight obstacles, gates/lifts, elevation lines, and
//! full 3D reachability are ported in sibling modules or incrementally
//! added here as the pathfinder needs them. Sectors and sprite-occlusion
//! masks are wired up.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::geo2d::{self, BBox2D, GeoPoint2D, Vec2D, pt};

// ---------------------------------------------------------------------------
// SectorIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `FastFindGrid::level::sectors` (the flat grid sector
/// table).  Wraps [`nonmax::NonMaxU32`] so `Option<SectorIndex>` is
/// 4 bytes via niche optimization.  Distinct from a sector *number*
/// (the script-facing `i16` id) and from `BuildingIdx` (which indexes
/// the building table); this is the FastFindGrid array slot.
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
pub struct SectorIndex(pub nonmax::NonMaxU32);

impl SectorIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<SectorIndex> for u32 {
    #[inline]
    fn from(i: SectorIndex) -> u32 {
        i.0.get()
    }
}
impl From<SectorIndex> for usize {
    #[inline]
    fn from(i: SectorIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for SectorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

// ─── Constants ───────────────────────────────────────────────────

/// Size of one grid cell in pixels (both X and Y).
pub const GRID_CELL_SIZE: i32 = 64;
/// As a float for arithmetic.
pub const GRID_CELL_SIZE_F: f32 = 64.0;

/// Grid cell diagonal vector (used for bounding box of a cell).
pub const GRID_CELL_DIAGONAL: Vec2D = Vec2D { x: 64.0, y: 64.0 };

// ─── QueryVisited ────────────────────────────────────────────────

/// Per-query O(1) dedup set for block scans.
struct QueryVisited {
    slots: Vec<bool>,
}

impl QueryVisited {
    fn new(len: usize) -> Self {
        Self {
            slots: vec![false; len],
        }
    }

    /// Returns `true` the first time `idx` is seen in this query.
    #[inline]
    fn try_mark(&mut self, idx: usize) -> bool {
        if self.slots[idx] {
            false
        } else {
            self.slots[idx] = true;
            true
        }
    }
}

// ─── Tactical point types ───────────────────────────────────────

pub const TACTICAL_RALLY_PT: u8 = 0;
pub const TACTICAL_SKIRT_AREA: u8 = 1;
pub const TACTICAL_HIDE_LINE: u8 = 2;
pub const TACTICAL_AMBUSH_PT: u8 = 3;
pub const TACTICAL_SIESTA_PT: u8 = 4;
pub const TACTICAL_CORNER_PT: u8 = 5;
pub const TACTICAL_SEEK_PT: u8 = 6;
pub const TACTICAL_HORSE_PARKING: u8 = 7;

// ─── Level-authored repulsive corner points ──────────────────────

/// Cone-limited repulsive point at an outward-facing sector corner.
/// Populated at level load: each concave corner of a walkable motion
/// area (or convex corner of an obstacle) gets a point that pushes
/// actors away from the corner along the angle bisector.
///
/// The "action field" limits the push to the corner's outward wedge —
/// outside that wedge the corner doesn't contribute.  `is_concave` is
/// set when the wedge spans more than 180°.
#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct LevelRepulsivePoint {
    pub position: MapPoint,
    pub layer: u16,
    /// Outward normal of the incoming-edge vector.
    pub limit_left: Vec2D,
    /// Outward normal of the outgoing-edge vector.
    pub limit_right: Vec2D,
    pub is_concave: bool,
}

// ─── Impact type ─────────────────────────────────────────────────

/// Type of impact when a projectile hits an obstacle.
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
)]
pub enum ImpactType {
    #[default]
    None,
    Ground,
    Wall,
    Top,
    Bottom,
}

// ─── Grid line ───────────────────────────────────────────────────

// ---------------------------------------------------------------------------
// LineIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `FastFindGrid::level::lines` (motion / elevation / repulsive
/// grid lines).  Wraps [`nonmax::NonMaxU32`] so `Option<LineIndex>` is
/// 4 bytes via niche optimization.  Distinct from [`crate::jump_line::JumpLineIndex`]
/// which indexes `level::jump_lines`.
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
pub struct LineIndex(pub nonmax::NonMaxU32);

impl LineIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<LineIndex> for u32 {
    #[inline]
    fn from(i: LineIndex) -> u32 {
        i.0.get()
    }
}
impl From<LineIndex> for usize {
    #[inline]
    fn from(i: LineIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for LineIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

/// A line stored in the grid, representing the properties of a level
/// line that are relevant for pathfinding collision queries.
///
/// We store the geometric data and flags directly rather than pointers
/// to a richer line object.
///
/// Pure static data — the runtime `active` toggle (set by patches) lives
/// in [`FastFindGrid::line_active`] and is keyed by line index.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GridLine {
    /// Segment endpoints.
    pub a: MapPoint,
    pub b: MapPoint,
    /// Whether this is a motion-blocking line (`LINE_MOTION`).
    pub is_motion: bool,
    /// Whether this is a repulsive line (`LINE_REPULSIVE`) — motion-area
    /// perimeters add these so the anti-collision step nudges actors
    /// away from walls instead of letting them slide against them.
    pub is_repulsive: bool,
    /// Whether this is an elevation (bond) line (`LINE_ELEVATION`).
    /// Elevation lines separate adjacent sight obstacles on the same
    /// layer; when an actor crosses one, its obstacle pointer swaps from
    /// `left_obstacle_index` to `right_obstacle_index` (or vice versa).
    pub is_elevation: bool,
    /// For elevation lines: sight obstacle index on the left side of the
    /// oriented segment (from `a` to `b`). `None` for non-elevation lines.
    pub left_obstacle_index: Option<u16>,
    /// For elevation lines: sight obstacle index on the right side of the
    /// oriented segment. `None` for non-elevation lines.
    pub right_obstacle_index: Option<u16>,
    /// Whether this is a patch-boundary line (`LINE_PATCH` | `LINE_CROSS`),
    /// constructed for `SECTOR_CROSS | SECTOR_PATCH` sectors (i.e. the
    /// apply / no-apply polygons carried on a patch). When a PC crosses
    /// this line on movement, the engine dispatches the patch enter/apply
    /// flow against [`Self::patch_index`].
    pub is_patch: bool,
    /// For patch lines: index of the owning patch in `EngineInner::patches`.
    /// `None` for non-patch lines.
    pub patch_index: Option<crate::patch::PatchIndex>,
    /// Whether this is a script-triggered boundary line (`LINE_SCRIPT`),
    /// constructed for any non-motion script-sector polygon edge. When an
    /// actor crosses this line, the engine fires the owning script zone's
    /// cross-sector hook.
    pub is_script: bool,
    /// For script lines: index into `EngineInner::script_zone_data` for the
    /// owning script zone. `None` when the line has no script association.
    pub script_zone_index: Option<u16>,
    /// Whether this is a sound-material boundary line (`LINE_SOUND`),
    /// constructed for any non-motion sound-sector polygon edge.  When an
    /// actor crosses this line, the engine refreshes the actor's
    /// `material` field from the new SECTOR_SOUND polygon containment
    /// (or falls back to the obstacle / default material).
    pub is_sound: bool,
    /// Outward normal (for repulsive lines, used in `FindAutorizedPosition`).
    pub normal: Vec2D,
    /// Bounding box of the segment (pre-computed for fast rejection).
    pub bbox: BBox2D,
}

impl GridLine {
    /// Create a new grid line from two endpoints.
    pub fn new(a: MapPoint, b: MapPoint, is_motion: bool) -> Self {
        let mut bbox = BBox2D::new();
        bbox.expand_point(a.to_geo());
        bbox.expand_point(b.to_geo());
        // Compute outward normal (perpendicular to segment, normalized).
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        let normal = if len > 1e-9 {
            pt(-dy / len, dx / len)
        } else {
            pt(0.0, 0.0)
        };
        Self {
            a,
            b,
            is_motion,
            is_repulsive: false,
            is_elevation: false,
            is_patch: false,
            patch_index: None,
            is_script: false,
            script_zone_index: None,
            is_sound: false,
            left_obstacle_index: None,
            right_obstacle_index: None,
            normal,
            bbox,
        }
    }

    /// Create a new patch-boundary line (`LINE_PATCH | LINE_CROSS`).
    ///
    /// Built for `IsPatch() && !IsMouse()` sectors. The `patch_index`
    /// carries the owning patch so the crossing dispatch in
    /// `check_for_patch_line_crossing` can route into `Patch::enter` /
    /// `apply` without a reverse lookup.  Not motion-blocking — LINE_PATCH
    /// is purely a trigger surface.
    pub fn new_patch(a: MapPoint, b: MapPoint, patch_index: crate::patch::PatchIndex) -> Self {
        let mut line = Self::new(a, b, false);
        line.is_patch = true;
        line.patch_index = Some(patch_index);
        line
    }

    /// Enable / disable the `LINE_REPULSIVE` flag.
    pub fn set_repulsive(&mut self, repulsive: bool) {
        self.is_repulsive = repulsive;
    }

    /// Recompute the repulsive-line normal for a motion sector edge.
    /// Walkable AREA sectors and solid motion obstacles use opposite
    /// orientations so the normal always points outward (away from the
    /// walkable side).
    pub fn initialize_motion_normal(&mut self, is_area: bool) {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        let len = (dx * dx + dy * dy).sqrt();
        self.normal = if len > 1e-9 {
            if is_area {
                pt(-dy / len, dx / len)
            } else {
                pt(dy / len, -dx / len)
            }
        } else {
            pt(0.0, 0.0)
        };
    }

    /// Create a new script-triggered boundary line
    /// (`LINE_SCRIPT | LINE_CROSS`).
    ///
    /// The `script_zone_index` stands in for the owning script-sector
    /// pointer — a populated value means the line has been associated
    /// to a script zone, `None` means the post-ctor state before any
    /// association call. Not motion-blocking; a script line is purely
    /// a trigger surface.
    pub fn new_script(a: MapPoint, b: MapPoint, script_zone_index: u16) -> Self {
        let mut line = Self::new(a, b, false);
        line.is_script = true;
        line.script_zone_index = Some(script_zone_index);
        line
    }

    /// Create a new sound-material boundary line (`LINE_SOUND | LINE_CROSS`).
    ///
    /// Built per SECTOR_SOUND polygon edge by
    /// [`FastFindGrid::add_sector_lines_for_sound`].  Not motion-blocking
    /// — purely a trigger surface that fires the actor-side material
    /// refresh when crossed.  The actual material refresh re-queries
    /// `MaterialSectors::material_at(actor_pos)` which combines the
    /// polygon containment test with the obstacle/default fallback in
    /// one call.
    pub fn new_sound(a: MapPoint, b: MapPoint) -> Self {
        let mut line = Self::new(a, b, false);
        line.is_sound = true;
        line
    }

    /// Create a new elevation (bond) line.
    ///
    /// A non-motion-blocking line separating two sight obstacles on one
    /// layer. `left_obstacle_index` / `right_obstacle_index` are the
    /// indices into `EngineInner::sight_obstacles` for the obstacles on the
    /// left/right of the oriented segment.
    pub fn new_elevation(
        a: MapPoint,
        b: MapPoint,
        left_obstacle_index: Option<u16>,
        right_obstacle_index: Option<u16>,
    ) -> Self {
        let mut line = Self::new(a, b, false);
        line.is_elevation = true;
        line.left_obstacle_index = left_obstacle_index;
        line.right_obstacle_index = right_obstacle_index;
        line
    }

    /// The line as a geo segment.
    #[inline]
    pub fn segment(&self) -> geo::Line<f32> {
        geo2d::segment(self.a.to_geo(), self.b.to_geo())
    }

    /// Test if this line's segment intersects another segment.
    #[inline]
    pub fn intersects_segment(&self, seg: geo::Line<f32>) -> bool {
        geo2d::segments_intersect(self.segment(), seg)
    }

    /// Test if this line's segment intersects a bounding box.
    pub fn intersects_bbox(&self, bbox: &BBox2D) -> bool {
        if let (Some(line_rect), Some(box_rect)) = (self.bbox.0, bbox.0) {
            use geo::Intersects;
            if !line_rect.intersects(&box_rect) {
                return false;
            }
            box_rect.intersects(&self.segment())
        } else {
            false
        }
    }
}

// ─── Grid sector (per-sector data for spatial queries) ──────────

/// Lightweight sector data stored in the grid for spatial hit-testing.
///
/// We store the minimal data needed for `get_sector` / `get_sector_screen`
/// queries: polygon vertices for point-in-polygon tests, type flags for
/// filtering, and references to related objects (doors, lifts).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GridSector {
    /// Polygon vertices defining this sector's boundary.
    pub points: Vec<MapPoint>,
    /// Axis-aligned bounding box (pre-computed for fast rejection).
    pub bounding_box: BBox2D,
    /// Bitflag combination of sector type properties (AREA, MOTION, DOOR, LIFT, etc.).
    pub sector_type: crate::sector::SectorType,
    /// Layer this sector belongs to.
    pub layer: u16,
    /// Global sector number.
    pub sector_number: crate::sector::SectorNumber,
    /// For door sectors: index into the global door table (`GameHost::doors`).
    pub door_index: Option<u32>,
    /// For lift sectors: the sub-type (wall/stairs/ladder/normal).
    pub lift_type: Option<crate::sector::LiftType>,
    /// For lift sectors: the proto-loaded facing direction (0..15).
    /// Used by climbing animations so the actor faces the lift's stored
    /// direction.
    pub lift_direction: i16,
    /// Whether all movement in this sector must be crouched.
    /// Loaded from proto motion area flags.
    pub force_crouched: bool,
    /// For building interior sectors: index into `GameHost::building_occupants`.
    pub building_index: Option<crate::sector::BuildingIdx>,

    // The runtime `active` toggle (set by patches) and per-lift occupancy /
    // wait-time live in [`FastFindGrid::sector_active`] and
    // [`FastFindGrid::lift_state`], keyed by sector index.

    // ── Lift endpoints (lifts only, populated at level load) ──
    /// Bottom lift-side exit point, mirroring legacy implementation `RHSectorLift::GetLowExitPoint`.
    pub low_exit_point: Option<MapPoint>,
    /// Top lift-side exit point, mirroring legacy implementation `RHSectorLift::GetHighExitPoint`.
    pub high_exit_point: Option<MapPoint>,

    // Door indices are ancillary references for code that still needs the
    // original door record (for example falling out of a lift to the outside
    // door point). Movement animation must use the direct exit points above.
    /// Index into `GameHost::doors` of the door at the bottom of this
    /// lift, or `None` for non-lift sectors. The door whose `point_in.Y`
    /// is largest (screen coords, Y grows downward). Used by
    /// `translate_ladder_wall_fall` to locate the ground entry point.
    pub lowest_door_index: Option<u32>,

    /// Indices into `FastFindGrid::jump_lines` of the jump lines that
    /// live in this sector. Each motion-area sector keeps a list of its
    /// own jump lines so the nearest-jump-line query can iterate without
    /// a global scan. Populated by `load_jump_lines_from_proto` during
    /// level init.
    pub jump_line_indices: Vec<crate::jump_line::JumpLineIndex>,

    /// Indices into `GameHost::doors` of the gates attached to this motion
    /// area sector. Populated during level load from each door's
    /// `sector_in` / `sector_out`. Used by the motion-area cursor
    /// reachability check and for gate-connectivity queries.
    pub gate_indices: Vec<crate::gate::DoorIndex>,

    /// For jump sectors (`SectorType::JUMP`): the index in
    /// `FastFindGrid::sectors` of the motion-area sector this jump
    /// polygon overlays.  When the cursor picks a jump sector but the
    /// click doesn't line up with any jumpable line, the cursor code
    /// recurses with the underlying sector.
    pub underlying_sector: Option<SectorIndex>,
}

impl GridSector {
    /// Point-in-polygon test using ray casting.
    pub fn contains_point(&self, pt: MapPoint) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        let pt_geo = pt.to_geo();
        if !self.bounding_box.contains_point(pt_geo) {
            return false;
        }
        let mut inside = false;
        let n = self.points.len();
        let mut j = n - 1;
        for i in 0..n {
            let vi = self.points[i];
            let vj = self.points[j];
            if (vi.y > pt.y) != (vj.y > pt.y) {
                let x_intersect = (vj.x - vi.x) * (pt.y - vi.y) / (vj.y - vi.y) + vi.x;
                if pt.x < x_intersect {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Polygon-vs-bounding-box intersection test.
    ///
    /// True iff any polygon point lies inside the box, any polygon
    /// edge (including the closing edge) intersects the box, or the
    /// box's top-left corner lies inside the polygon (box fully
    /// contained).
    pub fn intersects_bbox(&self, bbox: &BBox2D) -> bool {
        let n = self.points.len();
        if n == 0 {
            return false;
        }
        if n == 1 {
            return bbox.contains_point(self.points[0].to_geo());
        }

        let rect = match bbox.0 {
            Some(r) => r,
            None => return false,
        };

        // Any polygon point inside the box?
        for p in &self.points {
            if bbox.contains_point(p.to_geo()) {
                return true;
            }
        }

        // Any polygon edge (including closing) intersects the box?
        use geo::Intersects;
        let mut j = n - 1;
        for i in 0..n {
            let seg = geo::Line::new(self.points[j].to_geo(), self.points[i].to_geo());
            if rect.intersects(&seg) {
                return true;
            }
            j = i;
        }

        // Box fully contained in polygon — test top-left corner.
        self.contains_point(MapPoint::new(rect.min().x, rect.min().y))
    }
}

/// Result of a single-layer sector query (`get_sector`).
#[derive(Debug, Clone, Copy)]
pub enum SectorHit {
    /// Point is inside a sector.
    Found {
        sector_idx: SectorIndex,
        sector_number: crate::sector::SectorNumber,
    },
    /// Point is inside a motion obstacle (not walkable).
    Blocked,
    /// No sector contains this point on the queried layer.
    None,
}

/// Result of a multi-layer sector query (`get_sector_screen`).
#[derive(Debug, Clone, Copy)]
pub struct SectorScreenResult {
    /// Index into `FastFindGrid::sectors`, or `None` if no sector found.
    pub sector_idx: Option<SectorIndex>,
    /// Typed sector number, absent when no sector was found.
    pub sector: Option<crate::sector::SectorNumber>,
    /// Layer the sector was found on.
    pub layer: u16,
}

/// Projectile landing membership resolved from the projected landing
/// footprint. Picks the landing obstacle / layer, then walks motion
/// sectors in that grid block and accepts the containing motion area
/// unless a motion obstacle contains the point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileLandingResolution {
    pub obstacle_index: Option<crate::position_interface::ObstacleHandle>,
    pub obstacle_plane: Option<crate::position_interface::PlaneZCoeffs>,
    pub layer: u16,
    pub sector: Option<crate::position_interface::SectorHandle>,
    pub blocked_by_motion_obstacle: bool,
}

impl SectorScreenResult {
    /// Whether a valid sector was found.
    pub fn is_valid(&self) -> bool {
        self.sector_idx.is_some() && self.sector.is_some()
    }

    #[inline]
    fn found(
        sector_idx: SectorIndex,
        sector_number: crate::sector::SectorNumber,
        layer: u16,
    ) -> Self {
        Self {
            sector_idx: Some(sector_idx),
            sector: sector_number.is_valid().then_some(sector_number),
            layer,
        }
    }

    #[inline]
    fn invalid(layer: u16) -> Self {
        Self {
            sector_idx: None,
            sector: None,
            layer,
        }
    }

    /// Whether this is a valid position for a movement order.
    pub fn is_valid_for_move(&self, grid: &FastFindGrid) -> bool {
        let sector = match self.sector_idx {
            Some(idx) => match grid.level.sectors.get(usize::from(idx)) {
                Some(s) => s,
                None => return false,
            },
            None => return false,
        };
        let st = sector.sector_type;
        // A click is valid when it lands on a walkable area, a door,
        // or a jump overlay sector.
        (st.is_area() && st.is_motion()) || st.is_door() || st.is_jump()
    }
}

// ─── Grid block (per-cell storage) ──────────────────────────────

/// Per-cell storage of object indices.
///
/// We store indices into the grid's flat line, sector, mask, sight
/// obstacle, point and patch storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GridBlock {
    /// Indices into `FastFindGrid::lines`.
    pub line_indices: Vec<LineIndex>,
    /// Indices into `FastFindGrid::sectors`.
    pub sector_indices: Vec<u32>,
    /// Indices into `FastFindGrid::masks` (sprite-occlusion masks).
    pub mask_indices: Vec<crate::mask::MaskIndex>,
    /// Indices into `EngineInner::sight_obstacles` — the sight obstacles
    /// whose ground bounding box overlaps this cell. Populated by
    /// [`FastFindGrid::add_obstacle`] during level load.
    pub obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
}

// ─── Grid layer (per-layer storage) ─────────────────────────────

/// Per-layer storage of object indices.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GridLayer {
    /// All line indices belonging to this layer.
    pub line_indices: Vec<LineIndex>,
    /// All sector indices belonging to this layer.
    pub sector_indices: Vec<u32>,
    /// All mask indices belonging to this layer.
    pub mask_indices: Vec<crate::mask::MaskIndex>,
    /// All sight-obstacle indices belonging to this layer (or to every
    /// layer, for non-projection-area obstacles).
    pub obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
}

// ─── Static (level-loaded) grid data ─────────────────────────────

/// All level-loaded geometry that's read by the sim but never mutated
/// after `EngineInner::initialize_from_*` returns. Held as `Arc<LevelGrid>`
/// inside [`FastFindGrid::level`] so the per-frame rollback clone of
/// `EngineInner` is just a refcount bump rather than a 40+ MB deep copy of
/// every mask bitmap, line endpoint, and sector polygon.
///
/// This carries the static data side of the grid; the runtime flags
/// live on [`FastFindGrid`] itself. Construction only happens during
/// `EngineInner::initialize_from_*` via `Arc::make_mut`; once the level
/// is up the Arc gets shared with rollback snapshots and is never
/// mutated again.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct LevelGrid {
    /// Grid dimensions in cells.
    pub grid_width: u16,
    pub grid_height: u16,

    /// Index of the special layer (= number of conventional layers + 1 for lift layer).
    pub special_layer: u16,

    /// Map bounding box in world coordinates.
    pub map_bbox: BBox2D,

    /// All lines stored in the grid.
    pub lines: Vec<GridLine>,

    /// All sectors stored in the grid (flat arena).
    /// Indexed by `sector_indices` in grid blocks and layers.
    pub sectors: Vec<GridSector>,

    /// All sprite-occlusion masks stored in the grid (flat arena).
    /// Indexed by `mask_indices` in grid blocks and layers.
    pub masks: Vec<crate::mask::RuntimeMask>,

    /// All jump lines loaded from the proto JZ/PPPP chunk.
    /// Indexed by `jump_line_indices` on `GridSector`.  Loaded by
    /// `EngineInner::load_jump_lines_from_proto` after sectors are set up.
    pub jump_lines: Vec<crate::jump_line::JumpLine>,

    /// Grid blocks: indexed as `[x + grid_width * (y + layer * grid_height)]`.
    pub blocks: Vec<GridBlock>,

    /// Per-layer data.
    pub layers: Vec<GridLayer>,

    /// Move-box half-diagonals for each unit size.
    pub move_box_half_diagonals: Vec<Vec2D>,

    /// Lookup from sector_number (i16) to index in `sectors` vec.
    /// Populated during level loading for O(1) sector lookups by number.
    pub sector_number_map: std::collections::HashMap<crate::sector::SectorNumber, usize>,

    /// Cone-limited repulsive points at motion-sector / obstacle
    /// corners. Consulted by anti-collision via
    /// [`FastFindGrid::get_level_repulsive_points`].
    pub level_repulsive_points: Vec<LevelRepulsivePoint>,

    /// Per-shadow-sector centroid + radius. Keyed by `GridSector` index
    /// in [`Self::sectors`]. Populated by
    /// `EngineInner::initialize_shadow_sector_data` after the light-sector
    /// loop in `initialize_motion_from_level_data`, and only when ambience
    /// is NIGHT/FOG (the consumer paths early-out in any other ambience,
    /// so the per-shadow-sector init is gated on the same condition).
    /// Fully recomputed on each level load and serialized with the rest of
    /// the level-derived engine state.
    pub shadow_data: std::collections::HashMap<u32, crate::sector::ShadowData>,
}

// ─── Per-lift runtime state ──────────────────────────────────────

/// Mutable per-lift-sector state (occupancy + cooldown). Carved out of
/// `GridSector` so the geometry side stays purely static and rollback
/// snapshots only carry the small per-lift map (most sectors aren't
/// lifts and so have no entry).
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct LiftRuntimeState {
    pub occupants: u16,
    pub occupied_upwards: bool,
    pub occupied_downwards: bool,
    pub wait_time: u32,
}

impl LiftRuntimeState {
    /// Check if downward traversal is currently allowed.
    /// Decrements the wait timer as a side effect.
    pub fn is_authorized_downwards(&mut self) -> bool {
        if self.wait_time != 0 {
            self.wait_time -= 1;
            return false;
        }
        self.occupants == 0 || self.occupied_downwards
    }

    /// Check if upward traversal is currently allowed.
    /// Decrements the wait timer as a side effect.
    pub fn is_authorized_upwards(&mut self) -> bool {
        if self.wait_time != 0 {
            self.wait_time -= 1;
            return false;
        }
        self.occupants == 0 || self.occupied_upwards
    }

    /// Mark an actor as entering/leaving the lift going downwards.
    pub fn set_occupied_downwards(&mut self, entering: bool) {
        if entering {
            self.occupants += 1;
            self.occupied_downwards = true;
            self.wait_time = 100;
        } else {
            self.occupants = self.occupants.saturating_sub(1);
            if self.occupants == 0 {
                self.wait_time = 0;
                self.occupied_downwards = false;
                self.occupied_upwards = false;
            }
        }
    }

    /// Mark an actor as entering/leaving the lift going upwards.
    pub fn set_occupied_upwards(&mut self, entering: bool) {
        if entering {
            self.occupants += 1;
            self.occupied_upwards = true;
            self.wait_time = 80;
        } else {
            self.occupants = self.occupants.saturating_sub(1);
            if self.occupants == 0 {
                self.wait_time = 0;
                self.occupied_downwards = false;
                self.occupied_upwards = false;
            }
        }
    }
}

// ─── FastFindGrid ────────────────────────────────────────────────

/// The spatial acceleration grid for the game world.
///
/// Passed by reference to the pathfinder and other systems.
///
/// All the heavy level-loaded data (geometry, masks, blocks, layers)
/// lives behind [`Self::level`] in an [`Arc<LevelGrid>`] so cloning
/// `EngineInner` for rollback is a refcount bump rather than a multi-MB
/// deep copy. The fields directly on this struct are the runtime
/// per-element mutable flags + sparse overlays (cheap to clone, and
/// what `EngineSnapshot` actually carries).
#[derive(Debug)]
pub struct FastFindGrid {
    /// Static level-loaded grid data, shared with rollback snapshots.
    pub level: std::sync::Arc<LevelGrid>,

    // ── Runtime per-element flags ──
    //
    // These parallel-Vec arrays carry the runtime mutable state that
    // used to live on the geometry structs themselves (`GridLine.active`,
    // `GridSector.active`, `RuntimeMask.active`). They're length-paired
    // with `level.lines` / `level.sectors` / `level.masks` and indexed
    // by the same u32 index. Initialized to `true` on push by
    // `add_line` / `add_sector` / `add_mask`; set_*_active mutators
    // flip them.
    pub line_active: Vec<bool>,
    pub sector_active: Vec<bool>,
    pub mask_active: Vec<bool>,

    /// Sparse runtime state for lift sectors. Keyed by sector index;
    /// missing entry == default (zero occupants, zero wait time, not
    /// occupied). Most sectors aren't lifts so this map stays small.
    pub lift_state: std::collections::BTreeMap<u32, LiftRuntimeState>,

    /// Sparse runtime overlay onto `level.sectors[i].sector_type`.
    /// Currently only used by the ScApex script command (`script.rs`)
    /// which sets the APEX bit on a sector at runtime. Effective sector
    /// type = `level.sectors[i].sector_type | overlay.get(&i).copied().unwrap_or_default()`.
    pub sector_type_overlay: std::collections::BTreeMap<u32, crate::sector::SectorType>,
}

#[derive(Serialize)]
struct FastFindGridSnapshotRef<'a> {
    line_active: &'a [bool],
    sector_active: &'a [bool],
    mask_active: &'a [bool],
    lift_state: &'a std::collections::BTreeMap<u32, LiftRuntimeState>,
    sector_type_overlay: &'a std::collections::BTreeMap<u32, crate::sector::SectorType>,
}

#[derive(Deserialize)]
struct FastFindGridSnapshot {
    line_active: Vec<bool>,
    sector_active: Vec<bool>,
    mask_active: Vec<bool>,
    lift_state: std::collections::BTreeMap<u32, LiftRuntimeState>,
    sector_type_overlay: std::collections::BTreeMap<u32, crate::sector::SectorType>,
}

impl Serialize for FastFindGrid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        FastFindGridSnapshotRef {
            line_active: &self.line_active,
            sector_active: &self.sector_active,
            mask_active: &self.mask_active,
            lift_state: &self.lift_state,
            sector_type_overlay: &self.sector_type_overlay,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FastFindGrid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = FastFindGridSnapshot::deserialize(deserializer)?;
        Ok(Self {
            level: std::sync::Arc::new(LevelGrid::default()),
            line_active: snapshot.line_active,
            sector_active: snapshot.sector_active,
            mask_active: snapshot.mask_active,
            lift_state: snapshot.lift_state,
            sector_type_overlay: snapshot.sector_type_overlay,
        })
    }
}

impl robin_util::state_hash::StateHash for FastFindGrid {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.line_active.state_hash(state);
        self.sector_active.state_hash(state);
        self.mask_active.state_hash(state);
        self.lift_state.state_hash(state);
        self.sector_type_overlay.state_hash(state);
    }
}

impl Clone for FastFindGrid {
    fn clone(&self) -> Self {
        Self {
            level: self.level.clone(),
            line_active: self.line_active.clone(),
            sector_active: self.sector_active.clone(),
            mask_active: self.mask_active.clone(),
            lift_state: self.lift_state.clone(),
            sector_type_overlay: self.sector_type_overlay.clone(),
        }
    }
}

impl Default for FastFindGrid {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a flat block index from cell coordinates without borrowing
/// `&self`. Lets builder methods compute block indices while holding a
/// `&mut` borrow on the level Arc.
#[inline]
fn block_index_from_cell_raw(
    cx: u16,
    cy: u16,
    layer: u16,
    grid_width: u16,
    grid_height: u16,
) -> usize {
    (cx as usize)
        + (grid_width as usize) * ((cy as usize) + (layer as usize) * (grid_height as usize))
}

impl FastFindGrid {
    // ── Construction ──

    pub fn new() -> Self {
        Self {
            level: std::sync::Arc::new(LevelGrid::default()),
            line_active: Vec::new(),
            sector_active: Vec::new(),
            mask_active: Vec::new(),
            lift_state: std::collections::BTreeMap::new(),
            sector_type_overlay: std::collections::BTreeMap::new(),
        }
    }

    pub fn attach_level_grid(&mut self, level: std::sync::Arc<LevelGrid>) {
        self.level = level;
        if self.line_active.len() != self.level.lines.len() {
            panic!(
                "fast-grid line runtime length {} does not match level lines {}",
                self.line_active.len(),
                self.level.lines.len()
            );
        }
        if self.sector_active.len() != self.level.sectors.len() {
            panic!(
                "fast-grid sector runtime length {} does not match level sectors {}",
                self.sector_active.len(),
                self.level.sectors.len()
            );
        }
        if self.mask_active.len() != self.level.masks.len() {
            panic!(
                "fast-grid mask runtime length {} does not match level masks {}",
                self.mask_active.len(),
                self.level.masks.len()
            );
        }
    }

    /// Mutable access to the underlying static grid data via
    /// `Arc::make_mut`. Use only during level loading / builder code
    /// that's known to hold the only reference; after the level is up
    /// this would clone the Arc and silently lose the perf win.
    #[inline]
    pub fn level_mut(&mut self) -> &mut LevelGrid {
        std::sync::Arc::make_mut(&mut self.level)
    }

    /// Set the size of the map in grid cells.
    ///
    /// `height` is padded by +4 cells to match legacy grid behavior.
    pub fn size_map(&mut self, width: u16, height: u16) {
        let level = self.level_mut();
        level.grid_width = width;
        level.grid_height = height + 4; // +4-row pad inherited from level format
        level.map_bbox = BBox2D::from_coords(
            0.0,
            0.0,
            (width as f32) * GRID_CELL_SIZE_F - 1.0,
            (height as f32) * GRID_CELL_SIZE_F - 1.0,
        );
    }

    /// Allocate the block and layer arrays for the given number of layers.
    ///
    /// Must be called after `size_map` and before adding objects.
    pub fn allocate_layers(&mut self, num_conventional_layers: u16) {
        let level = self.level_mut();
        // Special layer = conventional layers + lift layer
        level.special_layer = num_conventional_layers + 1;
        let total_layers = (level.special_layer + 1) as usize;
        let total_blocks =
            total_layers * (level.grid_width as usize) * (level.grid_height as usize);

        level.blocks = vec![GridBlock::default(); total_blocks];
        level.layers = vec![GridLayer::default(); total_layers];
    }

    // ── Grid indexing ──

    /// Test if a projected map point is inside the grid.
    #[inline]
    pub fn is_inside_grid_point(&self, point: MapPoint) -> bool {
        self.is_inside_grid_point_geo(point.to_geo())
    }

    #[inline]
    fn is_inside_grid_point_geo(&self, point: GeoPoint2D) -> bool {
        let x = point.x as i32;
        let y = point.y as i32;
        x >= 0
            && x < (self.level.grid_width as i32) * GRID_CELL_SIZE
            && y >= 0
            && y < (self.level.grid_height as i32) * GRID_CELL_SIZE
    }

    /// Compute the flat block index for a world-space point on a given layer.
    #[inline]
    pub fn get_block_index(&self, point: MapPoint, layer: u16) -> usize {
        self.get_block_index_geo(point.to_geo(), layer)
    }

    #[inline]
    fn get_block_index_geo(&self, point: GeoPoint2D, layer: u16) -> usize {
        let bx = (point.x as i32) >> 6; // divide by 64
        let by = (point.y as i32) >> 6;
        (bx as usize)
            + (self.level.grid_width as usize)
                * ((by as usize) + (layer as usize) * (self.level.grid_height as usize))
    }

    /// Compute the flat block index from cell coordinates and layer.
    #[inline]
    pub fn block_index_from_cell(&self, cx: u16, cy: u16, layer: u16) -> usize {
        block_index_from_cell_raw(cx, cy, layer, self.level.grid_width, self.level.grid_height)
    }

    // ── Getters ──

    #[inline]
    pub fn lift_layer(&self) -> u16 {
        self.level.special_layer - 1
    }

    // ── Move box half-diagonals ──

    pub fn add_move_box_half_diagonal(&mut self, hd: Vec2D) {
        self.level_mut().move_box_half_diagonals.push(hd);
    }

    /// Safe lookup: returns `None` when the half-diagonal table hasn't
    /// been populated yet (e.g. pre-level-load) or the index is out of
    /// range.  Callers building a `PositionInterface` fall back to a
    /// unit-sized box in that case.
    pub fn try_move_box_half_diagonal(&self, index: usize) -> Option<Vec2D> {
        self.level.move_box_half_diagonals.get(index).copied()
    }

    // ── Line management ──

    /// Add a line to the grid. Returns the line index.
    pub fn add_line(&mut self, line: GridLine, layer: u16) -> LineIndex {
        let level = self.level_mut();
        let idx_u32 = level.lines.len() as u32;
        let idx = LineIndex::new(idx_u32).expect("line count exceeds u32::MAX - 1");
        // Add to layer
        if (layer as usize) < level.layers.len() {
            level.layers[layer as usize].line_indices.push(idx);
        }
        // Add to overlapping grid blocks
        if let Some(rect) = line.bbox.0 {
            let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_width.saturating_sub(1));
            let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_height.saturating_sub(1));

            for cy in y_min..=y_max {
                for cx in x_min..=x_max {
                    let block_idx = block_index_from_cell_raw(
                        cx,
                        cy,
                        layer,
                        level.grid_width,
                        level.grid_height,
                    );
                    if block_idx >= level.blocks.len() {
                        continue;
                    }
                    // Only register the line in cells its segment actually
                    // touches, not every cell in the bbox rectangle — avoids
                    // over-registering diagonals.
                    let block_bbox = BBox2D::from_coords(
                        f32::from(cx) * GRID_CELL_SIZE_F,
                        f32::from(cy) * GRID_CELL_SIZE_F,
                        f32::from(cx) * GRID_CELL_SIZE_F + GRID_CELL_SIZE_F,
                        f32::from(cy) * GRID_CELL_SIZE_F + GRID_CELL_SIZE_F,
                    );
                    if line.intersects_bbox(&block_bbox) {
                        level.blocks[block_idx].line_indices.push(idx);
                    }
                }
            }
        }
        level.lines.push(line);
        self.line_active.push(true);
        idx
    }

    /// Set the active state of a line.
    pub fn set_line_active(&mut self, line_idx: LineIndex, active: bool) {
        if let Some(slot) = self.line_active.get_mut(usize::from(line_idx)) {
            *slot = active;
        }
    }

    /// Set the active state of a sector.
    pub fn set_sector_active(&mut self, sector_idx: u32, active: bool) {
        if let Some(slot) = self.sector_active.get_mut(sector_idx as usize) {
            *slot = active;
        }
    }

    /// Set the active state of a mask.
    pub fn set_mask_active(&mut self, mask_idx: crate::mask::MaskIndex, active: bool) {
        if let Some(slot) = self.mask_active.get_mut(usize::from(mask_idx)) {
            *slot = active;
        }
    }

    /// Read the active state of a line. Returns `false` if the index is
    /// out of range.
    #[inline]
    pub fn is_line_active(&self, line_idx: LineIndex) -> bool {
        self.line_active
            .get(usize::from(line_idx))
            .copied()
            .unwrap_or(false)
    }

    /// Read the active state of a sector. Returns `false` if the index
    /// is out of range.
    #[inline]
    pub fn is_sector_active(&self, sector_idx: u32) -> bool {
        self.sector_active
            .get(sector_idx as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Read the active state of a mask. Returns `false` if the index is
    /// out of range.
    #[inline]
    pub fn is_mask_active(&self, mask_idx: crate::mask::MaskIndex) -> bool {
        self.mask_active
            .get(usize::from(mask_idx))
            .copied()
            .unwrap_or(false)
    }

    /// Mutable accessor to the lift runtime state. Inserts a default
    /// entry if absent.
    #[inline]
    pub fn lift_state_mut(&mut self, sector_idx: u32) -> &mut LiftRuntimeState {
        self.lift_state.entry(sector_idx).or_default()
    }

    /// Effective sector type for `sector_idx`: the loaded geometry
    /// value OR'd with any runtime overlay (currently just the APEX
    /// flag set by ScApex).
    #[inline]
    pub fn sector_type(&self, sector_idx: u32) -> crate::sector::SectorType {
        let base = self.level.sectors[sector_idx as usize].sector_type;
        match self.sector_type_overlay.get(&sector_idx) {
            Some(extra) => base | *extra,
            None => base,
        }
    }

    /// OR `extra` into the runtime overlay for `sector_idx`. Used by
    /// the ScApex script command.
    pub fn or_sector_type_overlay(&mut self, sector_idx: u32, extra: crate::sector::SectorType) {
        let entry = self
            .sector_type_overlay
            .entry(sector_idx)
            .or_insert_with(crate::sector::SectorType::empty);
        *entry |= extra;
    }

    // ── Sector management ──

    /// Add a sector to the grid. Returns the sector index.
    ///
    /// Registers the sector in the per-layer list and in every grid block
    /// that overlaps the sector's bounding box on the given layer.
    pub fn add_sector(&mut self, sector: GridSector, layer: u16) -> u32 {
        let level = self.level_mut();
        let idx = level.sectors.len() as u32;

        // Add to layer
        if (layer as usize) < level.layers.len() {
            level.layers[layer as usize].sector_indices.push(idx);
        }

        // Add to overlapping grid blocks. Each candidate block is gated
        // on a real polygon-vs-block-bbox intersection test, not just on
        // bbox overlap, so cells the polygon merely brushes by are not
        // registered.
        if let Some(rect) = sector.bounding_box.0 {
            let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_width.saturating_sub(1));
            let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_height.saturating_sub(1));

            for cy in y_min..=y_max {
                for cx in x_min..=x_max {
                    let block_idx = block_index_from_cell_raw(
                        cx,
                        cy,
                        layer,
                        level.grid_width,
                        level.grid_height,
                    );
                    if block_idx >= level.blocks.len() {
                        continue;
                    }
                    let cell_min = pt(
                        f32::from(cx) * GRID_CELL_SIZE_F,
                        f32::from(cy) * GRID_CELL_SIZE_F,
                    );
                    let cell_max = pt(cell_min.x + GRID_CELL_SIZE_F, cell_min.y + GRID_CELL_SIZE_F);
                    let block_box = BBox2D::from_corners(cell_min, cell_max);
                    if sector.intersects_bbox(&block_box) {
                        level.blocks[block_idx].sector_indices.push(idx);
                    }
                }
            }
        }

        // Populate sector_number → index lookup.
        let sn = sector.sector_number;
        level.sectors.push(sector);
        level.sector_number_map.insert(sn, idx as usize);
        self.sector_active.push(true);
        idx
    }

    // ── Mask management ──

    /// Add a sprite-occlusion mask to the grid.  Returns the mask index.
    ///
    /// Registers the mask in its layer's mask list and in every grid block
    /// overlapping its bounding box.
    pub fn add_mask(&mut self, mask: crate::mask::RuntimeMask) -> crate::mask::MaskIndex {
        let is_projectile = mask.is_projectile();
        let bbox = mask.bbox;
        let layer = mask.layer;

        let level = self.level_mut();
        let idx_u32 = level.masks.len() as u32;
        let idx = crate::mask::MaskIndex::new(idx_u32).expect("mask count exceeds u32::MAX - 1");

        if (layer as usize) < level.layers.len() {
            level.layers[layer as usize].mask_indices.push(idx);
        }

        if let Some(rect) = bbox.0 {
            let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_width.saturating_sub(1));
            let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_height.saturating_sub(1));

            for cy in y_min..=y_max {
                for cx in x_min..=x_max {
                    let block_idx = block_index_from_cell_raw(
                        cx,
                        cy,
                        layer,
                        level.grid_width,
                        level.grid_height,
                    );
                    if block_idx < level.blocks.len() {
                        level.blocks[block_idx].mask_indices.push(idx);
                    }
                }
            }
        }

        level.masks.push(mask);
        self.mask_active.push(true);

        // Projectile masks are registered a second time on the special
        // layer's grid blocks (but NOT the special-layer mask list) so
        // that `get_masks_applied_to_projectile(special_layer, …)` —
        // which queries the per-block mask_indices and is called by the
        // renderer in `game_render.rs` — sees them. The "skip the
        // per-layer list, only register in the per-block buckets" arm.
        if is_projectile {
            let special_layer = self.level.special_layer;
            self.add_mask_to_layer_blocks_only(idx, special_layer, bbox);
        }

        idx
    }

    /// Register an existing mask into the grid blocks of an additional
    /// layer, without touching any per-layer mask list.
    ///
    /// Used by the projectile duplicate-insert in `add_mask` — projectile
    /// masks live in their original layer's lists + blocks AND in the
    /// special layer's blocks so a special-layer block lookup finds them.
    fn add_mask_to_layer_blocks_only(
        &mut self,
        idx: crate::mask::MaskIndex,
        layer: u16,
        bbox: BBox2D,
    ) {
        let level = self.level_mut();
        let Some(rect) = bbox.0 else {
            return;
        };
        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(level.grid_height.saturating_sub(1));

        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx =
                    block_index_from_cell_raw(cx, cy, layer, level.grid_width, level.grid_height);
                if block_idx < level.blocks.len() {
                    level.blocks[block_idx].mask_indices.push(idx);
                }
            }
        }
    }

    // ── Sight-obstacle / point / patch indexing ──

    /// Register a sight-obstacle's external index into every grid block
    /// overlapping its ground bounding box, and into the per-layer list.
    ///
    /// Sight obstacles are owned by `EngineInner::sight_obstacles`; the
    /// grid only stores their index so spatial queries can jump straight
    /// to the right slice of obstacles without scanning the whole list.
    /// `layer == u16::MAX` is the convention for obstacles that apply to
    /// every layer (non-projection-area obstacles) — those are added only
    /// to the per-block lists.
    pub fn add_obstacle_index(
        &mut self,
        obstacle_idx: crate::sight_obstacle::SightObstacleIndex,
        layer: u16,
        bbox: &BBox2D,
    ) {
        let level = self.level_mut();
        if (layer as usize) < level.layers.len() {
            level.layers[layer as usize]
                .obstacle_indices
                .push(obstacle_idx);
        }
        let effective_layer = if (layer as usize) < level.layers.len() {
            layer
        } else {
            // Non-projection-area obstacles participate in every block
            // regardless of layer; pick layer 0 for the per-block entry
            // since our grid is layer-indexed and we can't replicate
            // across all layers cheaply.  The consumer-side type filter
            // compensates by treating layer==u16::MAX as "any".
            0
        };
        if let Some(rect) = bbox.0 {
            let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
            let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_width.saturating_sub(1));
            let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
                .min(level.grid_height.saturating_sub(1));
            for cy in y_min..=y_max {
                for cx in x_min..=x_max {
                    let block_idx = block_index_from_cell_raw(
                        cx,
                        cy,
                        effective_layer,
                        level.grid_width,
                        level.grid_height,
                    );
                    if block_idx < level.blocks.len() {
                        level.blocks[block_idx].obstacle_indices.push(obstacle_idx);
                    }
                }
            }
        }
    }

    /// Gather sight-obstacle indices whose ground bbox overlaps `bbox`
    /// on the given layer. Output is deduplicated. Used by 3D raycasts
    /// to restrict the obstacle scan to a spatial neighbourhood.
    pub fn get_obstacle_indices(
        &self,
        layer: u16,
        bbox: &BBox2D,
    ) -> Vec<crate::sight_obstacle::SightObstacleIndex> {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };
        if (layer as usize) >= self.level.layers.len() {
            return Vec::new();
        }
        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));
        let mut result: Vec<crate::sight_obstacle::SightObstacleIndex> = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &idx in &self.level.blocks[block_idx].obstacle_indices {
                    if !result.contains(&idx) {
                        result.push(idx);
                    }
                }
            }
        }
        result
    }

    /// Collect mask indices whose character polyline occludes a sprite
    /// centred at `position` with world-space bounding box `bbox` on the
    /// given `layer`.
    ///
    /// Iterates grid blocks overlapping `bbox`, deduplicates by index,
    /// and keeps only masks that are active, are character masks,
    /// intersect `bbox`, and report `is_applied_to_point_character(position)`.
    pub fn get_masks_applied_to_character(
        &self,
        layer: u16,
        bbox: &BBox2D,
        position: MapPoint,
    ) -> Vec<crate::mask::MaskIndex> {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };
        if (layer as usize) >= self.level.layers.len() {
            return Vec::new();
        }

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.masks.len());
        let mut result: Vec<crate::mask::MaskIndex> = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &mask_idx in &self.level.blocks[block_idx].mask_indices {
                    if !visited.try_mark(usize::from(mask_idx)) {
                        continue;
                    }
                    let mask = &self.level.masks[usize::from(mask_idx)];
                    if !self.is_mask_active(mask_idx) || !mask.is_character() {
                        continue;
                    }
                    if !mask.bbox.intersects_bbox(bbox) {
                        continue;
                    }
                    if mask.is_applied_to_point_character(position) {
                        result.push(mask_idx);
                    }
                }
            }
        }
        result
    }

    /// Resolve a projected mouse-map position to a 3D point on the topmost
    /// obstacle surface that contains it.
    ///
    /// Drops a vertical ray from far above the level onto the point and
    /// returns the first impact.  Composes
    /// [`crate::sight_obstacle::is_reachable_impact_fall_3d`] so the
    /// answer comes from the full 3D raycast path.
    ///
    /// `type_filter` selects which obstacle flag must be set (e.g.
    /// `SIGHTOBSTACLE_PROJECTION_AREA` for projectile aim).  Falls back
    /// to ground (`z = 0`) when no obstacle covers the point.
    pub fn convert_2d_to_3d(
        &self,
        pt2d: MapPoint,
        type_filter: u32,
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> crate::coordinates::WorldPoint3D {
        // The 2D point is a screen-Y coordinate in map space; to
        // deproject it onto any surface whose visible pixel sits at
        // `pt2d.y`, cast the isometric-projection ray
        // `(x, H, H - screenY) → (x, screenY, 0)` — every 3D world point
        // whose oblique projection's screen-Y equals `pt2d.y` satisfies
        // `Y - Z = pt2d.y`. When nothing is hit, fall back to the
        // ground-plane endpoint.
        //
        // Previously this compose used a vertical-drop
        // `is_reachable_impact_fall_3d`, which only hits obstacles whose
        // *ground* polygon contains the screen-space click. For tall
        // buildings / elevated rooftops that's typically not the same
        // obstacle as the visible top face, so ground-target throws
        // (purse, wasp nest) to a roof landed flat at `z = 0` instead
        // of on the roof.
        let world_height = (self.level.grid_height as f32) * GRID_CELL_SIZE_F;
        let origin = crate::coordinates::WorldPoint3D {
            x: pt2d.x,
            y: world_height,
            z: world_height - pt2d.y,
        };
        let destination = crate::coordinates::WorldPoint3D {
            x: pt2d.x,
            y: pt2d.y,
            z: 0.0,
        };
        crate::sight_obstacle::is_reachable_impact_3d(
            origin,
            destination,
            type_filter,
            obstacles,
            Some(self.level.map_bbox),
        )
        .map(|hit| hit.impact)
        .unwrap_or(destination)
    }

    /// Collect projectile-mask indices that occlude a flying entity at
    /// `position` (3D) with world-space bounding box `bbox`.
    ///
    /// Same grid walk as the character variant but uses the projectile
    /// polyline + 3D altitude check via
    /// [`crate::mask::RuntimeMask::is_applied_to_point_3d`].
    /// `is_human` distinguishes flying humans (bottom-plane test) from
    /// projectiles (top-plane test).
    pub fn get_masks_applied_to_projectile(
        &self,
        layer: u16,
        bbox: &BBox2D,
        position: crate::coordinates::WorldPoint3D,
        is_human: bool,
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> Vec<crate::mask::MaskIndex> {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };
        if (layer as usize) >= self.level.layers.len() {
            return Vec::new();
        }

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.masks.len());
        let mut result: Vec<crate::mask::MaskIndex> = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &mask_idx in &self.level.blocks[block_idx].mask_indices {
                    if !visited.try_mark(usize::from(mask_idx)) {
                        continue;
                    }
                    let mask = &self.level.masks[usize::from(mask_idx)];
                    if !self.is_mask_active(mask_idx) || !mask.is_projectile() {
                        continue;
                    }
                    if !mask.bbox.intersects_bbox(bbox) {
                        continue;
                    }
                    if mask.is_applied_to_point_3d(position, is_human, obstacles) {
                        result.push(mask_idx);
                    }
                }
            }
        }
        result
    }

    /// Get all active sectors matching `type_filter` at a given block index.
    pub fn get_sectors_at_block(
        &self,
        block_idx: usize,
        type_filter: crate::sector::SectorType,
    ) -> Vec<(u32, &GridSector)> {
        let block = match self.level.blocks.get(block_idx) {
            Some(b) => b,
            None => return Vec::new(),
        };
        let mut result = Vec::new();
        for &sector_idx in &block.sector_indices {
            if let Some(sector) = self.level.sectors.get(sector_idx as usize)
                && self.is_sector_active(sector_idx)
                && self.sector_type(sector_idx).contains(type_filter)
            {
                result.push((sector_idx, sector));
            }
        }
        result
    }

    /// True if `point` on `layer` falls inside an active `SECTOR_SHADOW`
    /// polygon. Used by the per-element refresh paths to suppress
    /// fog/night variant tinting when the actor stands in a shadow
    /// (=light-at-night) sector.
    pub fn is_in_shadow_sector(&self, point: MapPoint, layer: u16) -> bool {
        use crate::sector::SectorType;
        let block_idx = self.get_block_index(point, layer);
        self.get_sectors_at_block(block_idx, SectorType::SHADOW)
            .iter()
            .any(|(_, s)| s.contains_point(point))
    }

    /// Resolve the obstacle/layer/sector for a projectile that has just
    /// landed at `landing_screen` (screen-space `(x, y - z)`).
    ///
    /// The reference behavior starts from the impact obstacle when one
    /// was struck, then queries motion sectors at that layer to find the
    /// containing motion area. Call
    /// [`Self::resolve_projectile_landing_with_obstacle`] when the
    /// trajectory impact carried exact obstacle identity; this wrapper
    /// keeps the legacy screen-polygon fallback for callers that only
    /// know the landing footprint.
    pub fn resolve_projectile_landing(
        &self,
        landing_screen: MapPoint,
        sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> ProjectileLandingResolution {
        self.resolve_projectile_landing_with_obstacle(landing_screen, None, sight_obstacles)
    }

    /// Resolve projectile landing membership, preferring an exact
    /// obstacle index captured by the trajectory impact when available.
    /// This preserves the legacy implementation pointer identity behavior for overlapping
    /// projection areas; callers without trajectory obstacle identity
    /// still fall back to the screen polygon lookup.
    pub fn resolve_projectile_landing_with_obstacle(
        &self,
        landing_screen: MapPoint,
        exact_obstacle_index: Option<crate::position_interface::ObstacleHandle>,
        sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> ProjectileLandingResolution {
        use crate::sector::SectorType;

        let mut obstacle_index = None;
        let mut obstacle_plane = None;
        let mut layer = 0;

        let mut projection_iter: Box<
            dyn Iterator<Item = (u32, &crate::sight_obstacle::SightObstacle)> + '_,
        > = if let Some(handle) = exact_obstacle_index {
            let raw = u16::from(handle);
            Box::new(
                sight_obstacles
                    .get(raw as usize)
                    .into_iter()
                    .map(move |obstacle| (u32::from(raw), obstacle)),
            )
        } else {
            Box::new(sight_obstacles.iter_indexed())
        };

        for (idx, obstacle) in projection_iter.by_ref() {
            if !sight_obstacles.is_active(idx as usize)
                || !obstacle.is_projection_area()
                || !obstacle.contains_point_screen(landing_screen.to_geo())
            {
                continue;
            }
            if obstacle.layer == u16::MAX {
                tracing::warn!(
                    obstacle = idx,
                    "projection-area obstacle has no landing layer"
                );
                continue;
            }
            obstacle_index = u16::try_from(idx)
                .ok()
                .and_then(crate::position_interface::ObstacleHandle::new);
            obstacle_plane = Some(crate::position_interface::PlaneZCoeffs::from_plane_points(
                &obstacle.top_plane_points,
            ));
            layer = obstacle.layer;
            break;
        }

        let mut sector = None;
        if self.is_inside_grid_point(landing_screen) {
            let block_idx = self.get_block_index(landing_screen, layer);
            for (_, motion_sector) in self.get_sectors_at_block(block_idx, SectorType::MOTION) {
                if motion_sector.sector_type.is_area() {
                    if motion_sector.contains_point(landing_screen) {
                        sector = u16::try_from(i16::from(motion_sector.sector_number))
                            .ok()
                            .and_then(crate::position_interface::SectorHandle::new);
                    }
                } else if motion_sector.contains_point(landing_screen) {
                    return ProjectileLandingResolution {
                        obstacle_index,
                        obstacle_plane,
                        layer,
                        sector: None,
                        blocked_by_motion_obstacle: true,
                    };
                }
            }
        }

        ProjectileLandingResolution {
            obstacle_index,
            obstacle_plane,
            layer,
            sector,
            blocked_by_motion_obstacle: false,
        }
    }

    // ── Sector spatial queries ──

    /// Find which sector a point belongs to on a specific layer.
    ///
    /// Priority: door > lift > motion area (AREA). If the point is inside
    /// a motion obstacle (MOTION without AREA), returns `SectorHit::Blocked`.
    ///
    /// `reference` is used for jump-sector tie-breaking (closest to reference wins).
    pub fn get_sector(&self, pt: MapPoint, reference: MapPoint, layer: u16) -> SectorHit {
        use crate::sector::SectorType;

        if !self.is_inside_grid_point(pt) {
            return SectorHit::None;
        }
        let block_idx = self.get_block_index(pt, layer);
        let sectors = self.get_sectors_at_block(block_idx, SectorType::MOUSE);
        if sectors.is_empty() {
            return SectorHit::None;
        }

        // First pass: check special sectors (patch, lift, door) — they take priority.
        for &(idx, sector) in &sectors {
            if sector.sector_type.is_patch()
                && sector.contains_point(pt)
                && let Some(sector_idx) = SectorIndex::new(idx)
            {
                return SectorHit::Found {
                    sector_idx,
                    sector_number: sector.sector_number,
                };
            }
        }
        for &(idx, sector) in &sectors {
            if sector.sector_type.is_lift()
                && sector.contains_point(pt)
                && let Some(sector_idx) = SectorIndex::new(idx)
            {
                return SectorHit::Found {
                    sector_idx,
                    sector_number: sector.sector_number,
                };
            }
        }
        for &(idx, sector) in &sectors {
            if sector.sector_type.is_door()
                && sector.contains_point(pt)
                && let Some(sector_idx) = SectorIndex::new(idx)
            {
                return SectorHit::Found {
                    sector_idx,
                    sector_number: sector.sector_number,
                };
            }
        }

        // Sectors filtered out of the motion/jump passes: any sector
        // with a patch/lift/door flag that failed its containment test
        // above is dropped and never re-enters the remaining passes.
        // Sectors of those types that contained the point were already
        // returned. So the surviving list is precisely sectors without
        // any of the three special flags.
        let is_special = |s: &GridSector| {
            s.sector_type.is_patch() || s.sector_type.is_lift() || s.sector_type.is_door()
        };

        // Second pass: check motion sectors.
        let mut area_hit: Option<(u32, crate::sector::SectorNumber)> = None;
        for &(idx, sector) in &sectors {
            if is_special(sector) {
                continue;
            }
            if sector.sector_type.is_motion() {
                if sector.sector_type.is_area() {
                    if sector.contains_point(pt) {
                        area_hit = Some((idx, sector.sector_number));
                    }
                } else {
                    // Obstacle (MOTION without AREA) — point is blocked
                    if sector.contains_point(pt) {
                        return SectorHit::Blocked;
                    }
                }
            }
        }

        // Third pass: jump sectors override the motion-area hit when
        // the cursor is inside one and the reference point is closer
        // to that jump's first jump line than to any other overlapping
        // jump sector's first line. Tie-breaks by squared distance from
        // the reference to each jump's first line midpoint.
        if area_hit.is_some() {
            let mut best: Option<(u32, crate::sector::SectorNumber, f32)> = None;
            for &(idx, sector) in &sectors {
                if is_special(sector) {
                    continue;
                }
                if !sector.sector_type.is_jump() {
                    continue;
                }
                if !sector.contains_point(pt) {
                    continue;
                }
                let Some(&jl_idx) = sector.jump_line_indices.first() else {
                    continue;
                };
                let Some(jl) = self.level.jump_lines.get(usize::from(jl_idx)) else {
                    continue;
                };
                let mid = jl.get_middle_point();
                let dx = mid.x - reference.x;
                let dy = mid.y - reference.y;
                let sq = dx * dx + dy * dy;
                if best.map(|(_, _, prev)| sq < prev).unwrap_or(true) {
                    best = Some((idx, sector.sector_number, sq));
                }
            }
            if let Some((jidx, jnum, _)) = best
                && let Some(sector_idx) = SectorIndex::new(jidx)
            {
                return SectorHit::Found {
                    sector_idx,
                    sector_number: jnum,
                };
            }
        }

        match area_hit {
            Some((idx, num)) => match SectorIndex::new(idx) {
                Some(sector_idx) => SectorHit::Found {
                    sector_idx,
                    sector_number: num,
                },
                None => SectorHit::None,
            },
            None => SectorHit::None,
        }
    }

    /// Find the sector under a projected map point, searching layers top-down.
    ///
    /// Legacy C++ calls this "screen", but the point is the map projection
    /// `(world.x, world.y - world.z)`, not viewport pixels.
    ///
    /// Iterates from the highest layer down to 0, returning the first hit.
    /// Resolves associated sectors to their targets (lifts / drawbridges).
    pub fn get_sector_screen(&self, pt: MapPoint, reference: MapPoint) -> SectorScreenResult {
        // Iterate layers top-down from `special_layer - 1`.
        for layer in (0..self.level.special_layer).rev() {
            match self.get_sector(pt, reference, layer) {
                SectorHit::Found {
                    sector_idx,
                    sector_number,
                } => {
                    return SectorScreenResult::found(sector_idx, sector_number, layer);
                }
                SectorHit::Blocked => {
                    // A motion-obstacle hit (sector_number == -1) ends
                    // the search: the loop exits immediately and returns
                    // -1 to the caller. Do not continue into lower layers.
                    return SectorScreenResult::invalid(layer);
                }
                SectorHit::None => continue,
            }
        }
        SectorScreenResult::invalid(0)
    }

    /// Find the topmost walkable/teleportable sector under a projected map
    /// point. Used by the F7 teleport cheat to pick the destination
    /// sector.
    ///
    /// Unlike [`Self::get_sector_screen`] this accepts only two
    /// classes of hit:
    /// 1. motion + area → return the hit directly
    /// 2. jump sector → swap to the jump's underlying sector and
    ///    return it on the hit's own layer
    ///
    /// Doors and motion obstacles are rejected (the teleport caller
    /// then bails on `0xFFFF`). An empty reference point is used so the
    /// jump tie-break falls back to distance from the origin rather
    /// than biasing toward any particular PC position.
    pub fn get_sector_screen_accessible(&self, pt: MapPoint) -> SectorScreenResult {
        let empty_reference = MapPoint::ZERO;
        for layer in (0..self.level.special_layer).rev() {
            let hit = self.get_sector(pt, empty_reference, layer);
            let SectorHit::Found {
                sector_idx,
                sector_number,
            } = hit
            else {
                // SectorHit::Blocked and SectorHit::None don't match
                // any of the three accepted cases; continue down.
                continue;
            };
            let sector = match self.level.sectors.get(usize::from(sector_idx)) {
                Some(s) => s,
                None => continue,
            };
            let st = sector.sector_type;
            // Case 1: motion + area.
            if sector_number.is_valid() && st.is_motion() && st.is_area() {
                return SectorScreenResult::found(sector_idx, sector.sector_number, layer);
            }
            // Case 2: jump → underlying sector on this hit's layer.
            if sector_number.get() > 0
                && st.is_jump()
                && let Some(target_idx) = sector.underlying_sector
                && let Some(target) = self.level.sectors.get(usize::from(target_idx))
            {
                return SectorScreenResult::found(target_idx, target.sector_number, sector.layer);
            }
            // None of the accepted branches matched on this layer —
            // fall through to the next iteration rather than
            // returning.
        }
        SectorScreenResult::invalid(0)
    }

    /// "Peek under the topmost sector" variant of
    /// [`Self::get_sector_screen`] used by shift-held mouse selection.
    ///
    /// Walks layers top-down, stashes the first non-empty hit, and
    /// returns the *second* hit if one is found. Falls back to the
    /// stashed first hit when no second hit exists, and returns an
    /// empty result when no sector is found on any layer. Called from
    /// the mouse-location update path when shift is held.
    pub fn get_sector_screen_hidden(
        &self,
        pt: MapPoint,
        reference: MapPoint,
    ) -> SectorScreenResult {
        let mut first_hit: Option<SectorScreenResult> = None;

        for layer in (0..self.level.special_layer).rev() {
            let hit = self.get_sector(pt, reference, layer);
            match (first_hit, hit) {
                // First pass: stash the topmost non-empty hit.
                (
                    None,
                    SectorHit::Found {
                        sector_idx,
                        sector_number,
                    },
                ) => {
                    first_hit = Some(SectorScreenResult::found(sector_idx, sector_number, layer));
                }
                (None, SectorHit::Blocked) => {
                    first_hit = Some(SectorScreenResult::invalid(layer));
                }
                (None, SectorHit::None) => {}
                // Second pass: the first hit is already stashed; a new
                // non-empty hit here is the "hidden" sector under the
                // top one — return it directly.
                (
                    Some(_),
                    SectorHit::Found {
                        sector_idx,
                        sector_number,
                    },
                ) => {
                    return SectorScreenResult::found(sector_idx, sector_number, layer);
                }
                (Some(_), SectorHit::Blocked) => {
                    return SectorScreenResult::invalid(layer);
                }
                (Some(_), SectorHit::None) => {}
            }
        }

        first_hit.unwrap_or_else(|| SectorScreenResult::invalid(0))
    }

    // ── Line spatial queries ──

    /// Collect indices of active motion lines that overlap a bounding box on a given layer.
    ///
    /// This is the core spatial query used by the pathfinder: iterates
    /// grid blocks and filters by motion + active.
    pub fn get_active_motion_line_indices(&self, layer: u16, bbox: &BBox2D) -> Vec<LineIndex> {
        let mut result = Vec::new();
        self.visit_active_motion_line_indices(layer, bbox, |idx| {
            result.push(idx);
            true
        });
        result
    }

    fn visit_active_motion_line_indices(
        &self,
        layer: u16,
        bbox: &BBox2D,
        mut visit: impl FnMut(LineIndex) -> bool,
    ) {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return,
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.lines.len());
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    let line = &self.level.lines[usize::from(line_idx)];
                    if line.is_motion
                        && self.is_line_active(line_idx)
                        && visited.try_mark(usize::from(line_idx))
                        && !visit(line_idx)
                    {
                        return;
                    }
                }
            }
        }
    }

    /// Collect cone-limited repulsive points on `layer` whose
    /// position is inside `box_future` — the obstacle / motion-area
    /// corner pushes created at level load.
    pub fn get_level_repulsive_points(
        &self,
        layer: u16,
        bbox: &BBox2D,
    ) -> Vec<&LevelRepulsivePoint> {
        self.level
            .level_repulsive_points
            .iter()
            .filter(|p| p.layer == layer && bbox.contains_point(p.position.to_geo()))
            .collect()
    }

    /// Collect indices of active repulsive (`is_repulsive`) lines that
    /// overlap a bounding box on a given layer. The anti-collision step
    /// uses this to fetch wall / sector-perimeter pushes so actors
    /// don't scrape along motion lines.
    pub fn get_active_repulsive_line_indices(&self, layer: u16, bbox: &BBox2D) -> Vec<LineIndex> {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.lines.len());
        let mut result = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    let line = &self.level.lines[usize::from(line_idx)];
                    if line.is_repulsive
                        && self.is_line_active(line_idx)
                        && visited.try_mark(usize::from(line_idx))
                    {
                        result.push(line_idx);
                    }
                }
            }
        }
        result
    }

    /// Collect indices of active elevation (`is_elevation`) lines whose
    /// segment is crossed by the movement segment from `old_pos` to
    /// `new_pos` on the given layer.
    ///
    /// The per-tick crossing query, restricted to elevation lines.
    /// The returned list deduplicates indices.
    pub fn get_crossing_elevation_line_indices(
        &self,
        layer: u16,
        old_pos: GeoPoint2D,
        new_pos: GeoPoint2D,
    ) -> Vec<LineIndex> {
        let mut bbox = BBox2D::new();
        bbox.expand_point(old_pos);
        bbox.expand_point(new_pos);
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.lines.len());
        let mut result = Vec::new();
        let movement = geo2d::segment(old_pos, new_pos);
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    if !visited.try_mark(usize::from(line_idx)) {
                        continue;
                    }
                    let line = &self.level.lines[usize::from(line_idx)];
                    if !line.is_elevation || !self.is_line_active(line_idx) {
                        continue;
                    }
                    if line.intersects_segment(movement) {
                        result.push(line_idx);
                    }
                }
            }
        }
        self.remove_duplicate_elevation_crossings(&mut result);
        self.remove_old_position_elevation_crossings(&mut result, old_pos);
        result
    }

    fn remove_duplicate_elevation_crossings(&self, indices: &mut Vec<LineIndex>) {
        let mut i = 0;
        while i < indices.len() {
            let Some(line) = self.level.lines.get(usize::from(indices[i])) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < indices.len() {
                let Some(other) = self.level.lines.get(usize::from(indices[j])) else {
                    j += 1;
                    continue;
                };
                let same_direction = line.a == other.a && line.b == other.b;
                let reverse_direction = line.a == other.b && line.b == other.a;
                if same_direction || reverse_direction {
                    indices.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }

    fn remove_old_position_elevation_crossings(
        &self,
        indices: &mut Vec<LineIndex>,
        old_pos: GeoPoint2D,
    ) {
        indices.retain(|&idx| {
            let Some(line) = self.level.lines.get(usize::from(idx)) else {
                return false;
            };
            let line_a = line.a.to_geo();
            let line_b = line.b.to_geo();
            let line_vec = line_b - line_a;
            let test_vec = old_pos - line_a;
            line_vec.x * test_vec.y - line_vec.y * test_vec.x != 0.0
        });
    }

    /// Return every active `LINE_PATCH` line whose segment the movement
    /// vector `(old_pos → new_pos)` intersects on `layer`. Used by the
    /// per-PC patch-crossing dispatch.
    pub fn get_crossing_patch_line_indices(
        &self,
        layer: u16,
        old_pos: GeoPoint2D,
        new_pos: GeoPoint2D,
    ) -> Vec<LineIndex> {
        let mut bbox = BBox2D::new();
        bbox.expand_point(old_pos);
        bbox.expand_point(new_pos);
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let movement = geo2d::segment(old_pos, new_pos);

        let mut visited = QueryVisited::new(self.level.lines.len());
        let mut result = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    if !visited.try_mark(usize::from(line_idx)) {
                        continue;
                    }
                    let line = &self.level.lines[usize::from(line_idx)];
                    if !line.is_patch || !self.is_line_active(line_idx) {
                        continue;
                    }
                    // Same `is_crossed` guard as elevation crossings —
                    // patches sit on the same crossing code path.
                    if geo2d::is_crossed(line.segment(), movement) {
                        result.push(line_idx);
                    }
                }
            }
        }
        result
    }

    /// Construct `LINE_PATCH | LINE_CROSS` boundary segments for a
    /// `SECTOR_CROSS | SECTOR_PATCH` (non-MOUSE) sector and register
    /// them in the grid via `add_line`.
    ///
    /// Walks the sector polygon `[last → current]` edge-pairs and, for
    /// each, builds a patch line carrying the owning patch index, then
    /// registers it. Returns the `LineIndex` of each line pushed so the
    /// caller can stash them for the patch's `old_/new_line_indices`
    /// toggle lists.
    pub fn add_sector_lines_for_patch(
        &mut self,
        sector_grid_idx: u32,
        layer: u16,
        patch_index: crate::patch::PatchIndex,
        sector_active: bool,
    ) -> Vec<LineIndex> {
        let (points, point_count) = {
            let Some(sector) = self.level.sectors.get(sector_grid_idx as usize) else {
                return Vec::new();
            };
            if sector.points.is_empty() {
                return Vec::new();
            }
            (sector.points.clone(), sector.points.len())
        };

        let mut indices = Vec::with_capacity(point_count);
        let mut last = points[point_count - 1];
        for &current in &points {
            let line = GridLine::new_patch(last, current, patch_index);
            let idx = self.add_line(line, layer);
            self.set_line_active(idx, sector_active);
            indices.push(idx);
            last = current;
        }
        indices
    }

    /// Build LINE_SOUND segments for one polygon (typically a
    /// SECTOR_SOUND material polygon) and register them in the grid.
    /// Invoked from the sight-obstacle initialization for every
    /// material sector listed in the SIGHT chunk.
    ///
    /// Walks the polygon `[last → current]` edge-pairs, builds a
    /// non-motion LINE_SOUND line for each, and registers it with
    /// `add_line` at the supplied layer (layer 0 for SIGHT-listed
    /// material polygons). Returns each line's `LineIndex` so the
    /// caller can stash the list for diagnostics.
    pub fn add_sector_lines_for_sound(
        &mut self,
        layer: u16,
        points: &[MapPoint],
        sector_active: bool,
    ) -> Vec<LineIndex> {
        if points.len() < 2 {
            return Vec::new();
        }
        let mut indices = Vec::with_capacity(points.len());
        let mut last = points[points.len() - 1];
        for &current in points {
            let line = GridLine::new_sound(last, current);
            let idx = self.add_line(line, layer);
            self.set_line_active(idx, sector_active);
            indices.push(idx);
            last = current;
        }
        indices
    }

    /// Return every active `LINE_SOUND` line whose segment the
    /// movement vector `(old_pos → new_pos)` intersects on `layer`.
    /// Used by the per-actor sound-crossing dispatch.
    pub fn get_crossing_sound_line_indices(
        &self,
        layer: u16,
        old_pos: GeoPoint2D,
        new_pos: GeoPoint2D,
    ) -> Vec<LineIndex> {
        let mut bbox = BBox2D::new();
        bbox.expand_point(old_pos);
        bbox.expand_point(new_pos);
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let movement = geo2d::segment(old_pos, new_pos);

        let mut visited = QueryVisited::new(self.level.lines.len());
        let mut result = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    if !visited.try_mark(usize::from(line_idx)) {
                        continue;
                    }
                    let line = &self.level.lines[usize::from(line_idx)];
                    if !line.is_sound || !self.is_line_active(line_idx) {
                        continue;
                    }
                    if geo2d::is_crossed(line.segment(), movement) {
                        result.push(line_idx);
                    }
                }
            }
        }
        result
    }

    /// Construct `LINE_SCRIPT | LINE_CROSS` boundary segments for a
    /// `SECTOR_SCRIPT` polygon and register them in the grid via
    /// `add_line`.
    ///
    /// Walks the sector polygon `[last → current]` edge-pairs and, for
    /// each, builds a script line associated to the owning script zone,
    /// then registers it. Not motion-blocking — LINE_SCRIPT is purely a
    /// trigger surface. The `script_zone_index` lets the actor
    /// line-crossing dispatch route into the owning zone's cross hook
    /// without a reverse lookup.
    pub fn add_sector_lines_for_script(
        &mut self,
        sector_grid_idx: u32,
        layer: u16,
        script_zone_index: u16,
        sector_active: bool,
    ) -> Vec<LineIndex> {
        let (points, point_count) = {
            let Some(sector) = self.level.sectors.get(sector_grid_idx as usize) else {
                return Vec::new();
            };
            if sector.points.is_empty() {
                return Vec::new();
            }
            (sector.points.clone(), sector.points.len())
        };

        let mut indices = Vec::with_capacity(point_count);
        let mut last = points[point_count - 1];
        for &current in &points {
            let line = GridLine::new_script(last, current, script_zone_index);
            let idx = self.add_line(line, layer);
            self.set_line_active(idx, sector_active);
            indices.push(idx);
            last = current;
        }
        indices
    }

    /// Collect active motion lines overlapping grid blocks that intersect
    /// either of two segments. Used by `IsReachableGrid` in the pathfinder.
    ///
    /// Each block is checked against both segments before its lines are
    /// collected — blocks the segments don't touch are skipped.
    pub fn get_active_motion_lines_for_segments(
        &self,
        layer: u16,
        seg1: geo::Line<f32>,
        seg2: geo::Line<f32>,
        bbox: &BBox2D,
    ) -> Vec<LineIndex> {
        let rect = match bbox.0 {
            Some(r) => r,
            None => return Vec::new(),
        };

        let x_min = ((rect.min().x / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let y_min = ((rect.min().y / GRID_CELL_SIZE_F).floor() as i16).max(0) as u16;
        let x_max = ((rect.max().x / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_width.saturating_sub(1));
        let y_max = ((rect.max().y / GRID_CELL_SIZE_F).floor() as u16)
            .min(self.level.grid_height.saturating_sub(1));

        let mut visited = QueryVisited::new(self.level.lines.len());
        let mut result = Vec::new();
        for cy in y_min..=y_max {
            for cx in x_min..=x_max {
                // Check if the cell box intersects either segment
                let cell_min = pt(cx as f32 * GRID_CELL_SIZE_F, cy as f32 * GRID_CELL_SIZE_F);
                let cell_bbox =
                    BBox2D::from_corners(cell_min, pt(cell_min.x + 64.0, cell_min.y + 64.0));
                let cell_rect = cell_bbox.0.unwrap();

                use geo::Intersects;
                if !cell_rect.intersects(&seg1) && !cell_rect.intersects(&seg2) {
                    // Rect<f32>.intersects(Line<f32>)
                    continue;
                }

                let block_idx = self.block_index_from_cell(cx, cy, layer);
                if block_idx >= self.level.blocks.len() {
                    continue;
                }
                for &line_idx in &self.level.blocks[block_idx].line_indices {
                    let line = &self.level.lines[usize::from(line_idx)];
                    if line.is_motion
                        && self.is_line_active(line_idx)
                        && visited.try_mark(usize::from(line_idx))
                    {
                        result.push(line_idx);
                    }
                }
            }
        }
        result
    }

    // ── Thick movement corridor ──

    /// Build the two parallel side segments and bounding box of a "thick"
    /// movement corridor from `p1` to `p2` for a unit with the given
    /// `half_diagonal`.
    ///
    /// Returns `None` if the points are the same (zero movement).
    /// Otherwise returns `(segment1, segment2, corridor_bbox)`.
    ///
    /// This matches the repeated corridor-construction pattern in
    /// `IsReachableThick`, `IsReachableGrid`, etc.
    pub fn build_thick_move_corridor(
        p1: GeoPoint2D,
        p2: GeoPoint2D,
        half_diagonal: Vec2D,
    ) -> Option<ThickMoveCorridor> {
        let move_vec = pt(p2.x - p1.x, p2.y - p1.y);

        // Shrink half-diagonal by 1 pixel.
        let hd = pt(half_diagonal.x - 1.0, half_diagonal.y - 1.0);

        let (seg1, seg2, mut bbox) = if move_vec.x == 0.0 {
            if move_vec.y > 0.0 {
                // Moving straight down
                let s1_a = pt(p1.x + hd.x, p1.y - hd.y);
                let s1_b = pt(p2.x + hd.x, p2.y + hd.y);
                let s2_a = pt(p2.x - hd.x, p2.y + hd.y);
                let s2_b = pt(p1.x - hd.x, p1.y - hd.y);
                let mut bb = BBox2D::new();
                bb.expand_point(s2_b); // top-left
                bb.expand_point(s1_b); // bottom-right
                (geo2d::segment(s1_a, s1_b), geo2d::segment(s2_a, s2_b), bb)
            } else if move_vec.y < 0.0 {
                // Moving straight up
                let s1_a = pt(p1.x - hd.x, p1.y + hd.y);
                let s1_b = pt(p2.x - hd.x, p2.y - hd.y);
                let s2_a = pt(p2.x + hd.x, p2.y - hd.y);
                let s2_b = pt(p1.x + hd.x, p1.y + hd.y);
                let mut bb = BBox2D::new();
                bb.expand_point(s1_b); // top-left
                bb.expand_point(s2_b); // bottom-right
                (geo2d::segment(s1_a, s1_b), geo2d::segment(s2_a, s2_b), bb)
            } else {
                return None; // Zero movement
            }
        } else {
            // Normalize so X is positive (swap points if needed)
            let (mp1, mp2, mv) = if move_vec.x < 0.0 {
                (p2, p1, pt(-move_vec.x, -move_vec.y))
            } else {
                (p1, p2, move_vec)
            };

            if mv.y > 0.0 {
                // Right + down
                let s1_a = pt(mp1.x + hd.x, mp1.y - hd.y);
                let s1_b = pt(mp2.x + hd.x, mp2.y - hd.y);
                let s2_a = pt(mp2.x - hd.x, mp2.y + hd.y);
                let s2_b = pt(mp1.x - hd.x, mp1.y + hd.y);
                let mut bb = BBox2D::new();
                bb.expand_point(pt(mp1.x - hd.x, mp1.y - hd.y));
                bb.expand_point(pt(mp2.x + hd.x, mp2.y + hd.y));
                (geo2d::segment(s1_a, s1_b), geo2d::segment(s2_a, s2_b), bb)
            } else if mv.y < 0.0 {
                // Right + up
                let s1_a = pt(mp1.x - hd.x, mp1.y - hd.y);
                let s1_b = pt(mp2.x - hd.x, mp2.y - hd.y);
                let s2_a = pt(mp2.x + hd.x, mp2.y + hd.y);
                let s2_b = pt(mp1.x + hd.x, mp1.y + hd.y);
                let mut bb = BBox2D::new();
                bb.expand_point(pt(mp1.x - hd.x, mp1.y + hd.y));
                bb.expand_point(pt(mp2.x + hd.x, mp2.y - hd.y));
                (geo2d::segment(s1_a, s1_b), geo2d::segment(s2_a, s2_b), bb)
            } else {
                // Right, horizontal (Y == 0) — special +/- 1 adjustment
                let s1_a = pt(mp1.x - hd.x - 1.0, mp1.y - hd.y);
                let s1_b = pt(mp2.x + hd.x - 1.0, mp2.y - hd.y);
                let s2_a = pt(mp2.x + hd.x - 1.0, mp2.y + hd.y);
                let s2_b = pt(mp1.x - hd.x - 1.0, mp1.y + hd.y);
                let mut bb = BBox2D::new();
                bb.expand_point(s1_a);
                bb.expand_point(s2_a);
                (geo2d::segment(s1_a, s1_b), geo2d::segment(s2_a, s2_b), bb)
            }
        };

        // Ensure bbox covers both segment endpoints
        bbox.expand_point(seg1.start);
        bbox.expand_point(seg1.end);
        bbox.expand_point(seg2.start);
        bbox.expand_point(seg2.end);

        // Compute corridor edge vectors for point-in-corridor test
        let vec1 = pt(seg1.end.x - seg1.start.x, seg1.end.y - seg1.start.y);
        let vec2 = pt(seg2.end.x - seg2.start.x, seg2.end.y - seg2.start.y);
        let vec3 = pt(seg2.start.x - seg1.end.x, seg2.start.y - seg1.end.y);
        let vec4 = pt(seg1.start.x - seg2.end.x, seg1.start.y - seg2.end.y);

        Some(ThickMoveCorridor {
            seg1,
            seg2,
            bbox,
            vec1,
            vec2,
            vec3,
            vec4,
        })
    }

    // ── Collision / Reachability ──

    /// Check if a bounding box does not intersect any active motion line.
    pub fn is_position_authorized(&self, bbox: &BBox2D, layer: u16) -> bool {
        // This gates only on the grid-block bounds; the pathfinder
        // wrapper (`PathFinder::object_position_authorized`) does the
        // out-of-grid rejection before invoking this. A previous
        // `map_bbox.intersects_bbox` precheck here diverged from that:
        // a bbox inside the grid but outside `map_bbox` would return
        // false where the line-intersection test should run instead.
        // Check actual intersection (not just block overlap)
        let mut authorized = true;
        self.visit_active_motion_line_indices(layer, bbox, |idx| {
            let line = &self.level.lines[usize::from(idx)];
            if line.intersects_bbox(bbox) {
                tracing::trace!(
                    ?bbox,
                    layer,
                    line_idx = idx.get(),
                    line_a = ?line.a,
                    line_b = ?line.b,
                    is_motion = line.is_motion,
                    "is_position_authorized: rejected by motion line"
                );
                authorized = false;
                return false;
            }
            true
        });
        authorized
    }

    /// Check if a thick (unit-sized) straight-line movement from `p1` to `p2`
    /// is clear of active motion lines.
    ///
    /// `half_diagonal` is the half-diagonal of the unit's bounding box.
    pub fn is_reachable_thick(
        &self,
        p1: MapPoint,
        p2: MapPoint,
        layer: u16,
        half_diagonal: Vec2D,
    ) -> bool {
        if p1 == p2 {
            return true;
        }

        let corridor =
            match Self::build_thick_move_corridor(p1.to_geo(), p2.to_geo(), half_diagonal) {
                Some(c) => c,
                None => return true,
            };

        let line_indices = self.get_active_motion_line_indices(layer, &corridor.bbox);
        if line_indices.is_empty() {
            return true;
        }

        // Check segment intersections
        for &idx in &line_indices {
            let line = &self.level.lines[usize::from(idx)];
            if line.intersects_segment(corridor.seg1) || line.intersects_segment(corridor.seg2) {
                return false;
            }
        }

        // Check if any line endpoint lies inside the corridor
        for &idx in &line_indices {
            let p = self.level.lines[usize::from(idx)].a;
            if corridor.point_inside(p.to_geo()) {
                return false;
            }
        }

        true
    }

    /// Check thin (zero-width) reachability between two points.
    pub fn is_reachable_thin(&self, p1: MapPoint, p2: MapPoint, layer: u16) -> bool {
        self.is_reachable_thin_geo(p1.to_geo(), p2.to_geo(), layer)
    }

    fn is_reachable_thin_geo(&self, p1: GeoPoint2D, p2: GeoPoint2D, layer: u16) -> bool {
        let seg = geo2d::segment(p1, p2);
        let mut bbox = BBox2D::new();
        bbox.expand_point(p1);
        bbox.expand_point(p2);

        let mut reachable = true;
        self.visit_active_motion_line_indices(layer, &bbox, |idx| {
            let line = &self.level.lines[usize::from(idx)];
            if line.intersects_segment(seg) {
                reachable = false;
                return false;
            }
            true
        });
        reachable
    }

    /// Check if a 3D trajectory segment passes through sight obstacles
    /// or ground-level motion lines.
    ///
    /// Walks sight-obstacle groups and tests ray-against-walls,
    /// ray-against-top-plane, ray-against-bottom-plane, and
    /// ray-against-ground. Composes
    /// [`crate::sight_obstacle::is_reachable_impact_3d`] — which already
    /// implements the 3D intersection math — and additionally checks
    /// the grid's active 2D motion lines on the ray's layer so that
    /// ground-level walls block the ray even when no sight obstacle
    /// covers the corridor. Returns `true` when the path is clear,
    /// `false` when blocked, writing the first impact to `impact` on
    /// a blocked ray.
    pub fn is_reachable_impact_3d(
        &self,
        origin: crate::coordinates::WorldPoint3D,
        destination: crate::coordinates::WorldPoint3D,
        layer: u16,
        type_filter: u32,
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
        impact: &mut Option<crate::sight_obstacle::ImpactResult3D>,
    ) -> bool {
        // 3D obstacle intersection (top plane, bottom plane, walls,
        // ground).
        let obstacle_hit = crate::sight_obstacle::is_reachable_impact_3d(
            origin,
            destination,
            type_filter,
            obstacles,
            Some(self.level.map_bbox),
        );

        // Ground-level motion line intersection: the corridor between
        // `origin` and `destination` projected onto the map. This is
        // the wall-collision check that the original folded into the
        // obstacle wall loop for on-ground obstacles. Motion lines live
        // on the fast grid and sight obstacles on the engine, so we
        // query each store separately and pick the earliest impact.
        let origin_2d = crate::geo2d::pt(origin.x, origin.y);
        let dest_2d = crate::geo2d::pt(destination.x, destination.y);
        let motion_t = self.impact_intersection_ratio(origin_2d, dest_2d, layer);

        // Pick the nearest impact and report it.  Return false (blocked)
        // if either path found one.
        match (obstacle_hit, motion_t) {
            (None, None) => {
                *impact = None;
                true
            }
            (Some(hit), None) => {
                *impact = Some(hit);
                false
            }
            (None, Some(t)) => {
                let (ix, iy) = (
                    origin.x + t * (destination.x - origin.x),
                    origin.y + t * (destination.y - origin.y),
                );
                *impact = Some(crate::sight_obstacle::ImpactResult3D {
                    impact: crate::coordinates::WorldPoint3D {
                        x: ix,
                        y: iy,
                        z: origin.z + t * (destination.z - origin.z),
                    },
                    obstacle_index: None,
                });
                false
            }
            (Some(hit), Some(t)) => {
                // Compute the obstacle hit's parameter along the ray
                // to compare distances in a common frame.
                let len = {
                    let dx = destination.x - origin.x;
                    let dy = destination.y - origin.y;
                    let dz = destination.z - origin.z;
                    (dx * dx + dy * dy + dz * dz).sqrt()
                };
                let t_obs = if len > 1e-9 {
                    let dx = hit.impact.x - origin.x;
                    let dy = hit.impact.y - origin.y;
                    let dz = hit.impact.z - origin.z;
                    (dx * dx + dy * dy + dz * dz).sqrt() / len
                } else {
                    0.0
                };
                if t <= t_obs {
                    let (ix, iy) = (
                        origin.x + t * (destination.x - origin.x),
                        origin.y + t * (destination.y - origin.y),
                    );
                    *impact = Some(crate::sight_obstacle::ImpactResult3D {
                        impact: crate::coordinates::WorldPoint3D {
                            x: ix,
                            y: iy,
                            z: origin.z + t * (destination.z - origin.z),
                        },
                        obstacle_index: None,
                    });
                } else {
                    *impact = Some(hit);
                }
                false
            }
        }
    }

    /// Boolean 3D reachability check for callers that do not need an
    /// impact point.
    ///
    /// The sight-obstacle side can early-return on the first blocker
    /// instead of computing the nearest impact ratio for every candidate.
    pub fn is_reachable_3d(
        &self,
        origin: crate::coordinates::WorldPoint3D,
        destination: crate::coordinates::WorldPoint3D,
        layer: u16,
        type_filter: u32,
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> bool {
        let origin_arr = [origin.x, origin.y, origin.z];
        let dest_arr = [destination.x, destination.y, destination.z];
        if !crate::sight_obstacle::is_reachable_3d(obstacles, origin_arr, dest_arr, type_filter) {
            return false;
        }

        let origin_2d = crate::geo2d::pt(origin.x, origin.y);
        let dest_2d = crate::geo2d::pt(destination.x, destination.y);
        self.impact_intersection_ratio(origin_2d, dest_2d, layer)
            .is_none()
    }

    /// Find the earliest intersection ratio along a trajectory segment
    /// against blocking motion lines.
    ///
    /// Returns a value in `0.0..=1.0` representing how far along the
    /// segment `origin→destination` the first obstacle intersection
    /// occurs.  Returns `None` if no intersection (path is clear).
    ///
    /// Uses parametric segment–segment intersection: for trajectory
    /// segment P and obstacle line Q, solves for t in `P(t) = P0 + t*(P1-P0)`
    /// where P intersects Q.
    pub fn impact_intersection_ratio(
        &self,
        origin: GeoPoint2D,
        destination: GeoPoint2D,
        layer: u16,
    ) -> Option<f32> {
        let seg = geo2d::segment(origin, destination);
        let mut bbox = BBox2D::new();
        bbox.expand_point(origin);
        bbox.expand_point(destination);

        let mut min_t: Option<f32> = None;

        let dx = destination.x - origin.x;
        let dy = destination.y - origin.y;

        self.visit_active_motion_line_indices(layer, &bbox, |idx| {
            let line = &self.level.lines[usize::from(idx)];
            if !line.intersects_segment(seg) {
                return true;
            }
            // Compute parametric t along the trajectory segment.
            // Using the standard 2D segment intersection formula:
            //   t = ((q - p) × s) / (r × s)
            // where p=origin, r=dest-origin, q=line.a, s=line.b-line.a
            let qx = line.a.x - origin.x;
            let qy = line.a.y - origin.y;
            let sx = line.b.x - line.a.x;
            let sy = line.b.y - line.a.y;

            let r_cross_s = dx * sy - dy * sx;
            if r_cross_s.abs() < 1e-9 {
                // Parallel — use t=0.5 as approximation for collinear overlap.
                let t = 0.5;
                min_t = Some(min_t.map_or(t, |prev: f32| prev.min(t)));
                return true;
            }

            let t = (qx * sy - qy * sx) / r_cross_s;
            let t = t.clamp(0.0, 1.0);
            min_t = Some(min_t.map_or(t, |prev: f32| prev.min(t)));
            true
        });

        min_t
    }

    /// Check if a straight thick movement from `p1` to `p2` is authorized.
    /// Combines position authorization at the destination with thick reachability.
    ///
    /// `move_box` is the 0-centered bounding box of the unit.
    pub fn is_straight_movement_authorized(
        &self,
        p1: MapPoint,
        p2: MapPoint,
        layer: u16,
        move_box: &BBox2D,
    ) -> bool {
        // Check destination position is authorized
        let dest_box = move_box.translated(pt(p2.x, p2.y));
        if !self.is_position_authorized(&dest_box, layer) {
            return false;
        }
        // Check thick corridor is clear
        let half_diag = pt(move_box.x_max(), move_box.y_max());
        self.is_reachable_thick(p1, p2, layer, half_diag)
    }

    /// Check if a thick straight-line movement from `p1` to `p2` is
    /// clear of static motion lines AND of caller-supplied mobile
    /// repulsive lines. Same thick-corridor segment math as
    /// [`Self::is_reachable_thick`], then also tests the corridor
    /// against every mobile `LINE_REPULSIVE` line (one per live mobile
    /// element).
    ///
    /// The mobile lines are passed in as a slice because mobile
    /// elements are owned by `EngineInner`, not by the grid — the
    /// engine builds the slice from its current tick's live mobile
    /// elements and hands it to the grid for the corridor test.
    /// Passing `&[]` reduces to pure static motion-line checks.
    pub fn is_reachable_thick_mobile(
        &self,
        p1: MapPoint,
        p2: MapPoint,
        layer: u16,
        half_diagonal: Vec2D,
        mobile_lines: &[GridLine],
    ) -> bool {
        if !self.is_reachable_thick(p1, p2, layer, half_diagonal) {
            return false;
        }
        if mobile_lines.is_empty() || p1 == p2 {
            return true;
        }
        let Some(corridor) =
            Self::build_thick_move_corridor(p1.to_geo(), p2.to_geo(), half_diagonal)
        else {
            return true;
        };
        for line in mobile_lines {
            // Mobile (per-entity repulsive) lines have no runtime active
            // toggle — they're rebuilt each tick from the live entity
            // set, so they're implicitly "always active."
            if !line.intersects_bbox(&corridor.bbox) {
                continue;
            }
            if line.intersects_segment(corridor.seg1) || line.intersects_segment(corridor.seg2) {
                return false;
            }
            if corridor.point_inside(line.a.to_geo()) {
                return false;
            }
        }
        true
    }

    /// Find an authorized (non-colliding) position for a bounding box
    /// by iteratively pushing it away from intersecting motion lines.
    ///
    /// Returns `true` if a valid position was found (modifying `bbox` in place).
    pub fn find_authorized_position(&self, bbox: &mut BBox2D, layer: u16) -> bool {
        // Defensive: an unset bbox has no center / corners to push
        // around and would panic on the `bbox.center()` call below.
        // With actors now properly populated with a move-box at spawn
        // (`PositionInterface::for_actor`) this shouldn't happen in
        // normal flow, but the guardrail returns `false` when the bbox
        // is null.
        if bbox.0.is_none() {
            return false;
        }

        // If outside map, clamp to map bounds first
        if !self.level.map_bbox.intersects_bbox(bbox)
            && let (Some(el_rect), Some(map_rect)) = (bbox.0, self.level.map_bbox.0)
        {
            let mut tx = 0.0f32;
            let mut ty = 0.0f32;

            if el_rect.max().x < 0.0 {
                tx = -el_rect.min().x;
            } else if el_rect.min().x > map_rect.max().x {
                tx = -(el_rect.max().x - map_rect.max().x);
            }

            if el_rect.max().y < 0.0 {
                ty = -el_rect.min().y;
            } else if el_rect.min().y > map_rect.max().y {
                ty = -(el_rect.max().y - map_rect.max().y);
            }

            bbox.translate(pt(tx, ty));
        }

        // Iteratively push away from motion lines (up to 50 tries).
        // Per-line push order and live re-read of center/corners: every
        // corner read after a translation sees the updated bbox, so
        // later corner pushes in the same line pass typically drop
        // below the `-0.1` gate and no-op.
        for _ in 0..50 {
            let line_indices = self.get_active_motion_line_indices(layer, bbox);
            let intersecting: Vec<LineIndex> = line_indices
                .into_iter()
                .filter(|&idx| self.level.lines[usize::from(idx)].intersects_bbox(bbox))
                .collect();

            if intersecting.is_empty() {
                return true;
            }

            for &idx in &intersecting {
                let line = &self.level.lines[usize::from(idx)];
                // Recompute center on every line iteration so a push
                // from an earlier line can flip this gate.
                let center = bbox.center();
                let to_center = pt(center.x - line.a.x, center.y - line.a.y);
                if geo2d::dot(line.normal, to_center) <= 0.0 {
                    continue;
                }
                push_corners_away_from_line(bbox, line);
            }
        }

        false
    }

    /// Find an authorized position by pushing the box away from lines,
    /// using `click` as the reference direction instead of box center.
    pub fn find_authorized_position_toward(
        &self,
        bbox: &mut BBox2D,
        click: MapPoint,
        layer: u16,
    ) -> bool {
        self.find_authorized_position_toward_geo(bbox, click.to_geo(), layer)
    }

    fn find_authorized_position_toward_geo(
        &self,
        bbox: &mut BBox2D,
        click: GeoPoint2D,
        layer: u16,
    ) -> bool {
        for _ in 0..50 {
            // Collect lines from both the segment (center→click) and the box itself
            let center = bbox.center();
            let seg_bbox = {
                let mut b = BBox2D::new();
                b.expand_point(center);
                b.expand_point(click);
                b
            };
            let seg_lines = self.get_active_motion_line_indices(layer, &seg_bbox);
            let box_lines = self.get_active_motion_line_indices(layer, bbox);

            // Merge and dedup
            let mut all_indices = seg_lines;
            for &idx in &box_lines {
                if !all_indices.contains(&idx) {
                    all_indices.push(idx);
                }
            }

            // Filter to actually intersecting lines
            let intersecting: Vec<LineIndex> = all_indices
                .into_iter()
                .filter(|&idx| {
                    let line = &self.level.lines[usize::from(idx)];
                    line.intersects_bbox(bbox)
                        || line.intersects_segment(geo2d::segment(center, click))
                })
                .collect();

            if intersecting.is_empty() {
                return true;
            }

            for &idx in &intersecting {
                let line = &self.level.lines[usize::from(idx)];
                // Push toward the click side of the line
                let to_click = pt(click.x - line.a.x, click.y - line.a.y);
                if geo2d::dot(line.normal, to_click) <= 0.0 {
                    continue;
                }
                push_corners_away_from_line(bbox, line);
            }
        }
        false
    }

    /// Find an authorized position along the segment from `start` to the box
    /// center, then refine by pushing away from the box itself.
    pub fn find_authorized_position_straight(
        &self,
        bbox: &mut BBox2D,
        start: MapPoint,
        layer: u16,
    ) -> bool {
        self.find_authorized_position_straight_geo(bbox, start.to_geo(), layer)
    }

    fn find_authorized_position_straight_geo(
        &self,
        bbox: &mut BBox2D,
        start: GeoPoint2D,
        layer: u16,
    ) -> bool {
        // Phase 1: push along segment (center → start)
        for _ in 0..50 {
            let center = bbox.center();
            let seg_bbox = {
                let mut b = BBox2D::new();
                b.expand_point(center);
                b.expand_point(start);
                b
            };
            let line_indices = self.get_active_motion_line_indices(layer, &seg_bbox);
            let intersecting: Vec<LineIndex> = line_indices
                .into_iter()
                .filter(|&idx| {
                    self.level.lines[usize::from(idx)]
                        .intersects_segment(geo2d::segment(center, start))
                })
                .collect();

            if intersecting.is_empty() {
                break;
            }

            for &idx in &intersecting {
                let line = &self.level.lines[usize::from(idx)];
                let to_start = pt(start.x - line.a.x, start.y - line.a.y);
                if geo2d::dot(line.normal, to_start) <= 0.0 {
                    continue;
                }
                push_corners_away_from_line(bbox, line);
            }
        }

        // Phase 2: push away from box-overlapping lines
        for _ in 0..50 {
            let line_indices = self.get_active_motion_line_indices(layer, bbox);
            let intersecting: Vec<LineIndex> = line_indices
                .into_iter()
                .filter(|&idx| self.level.lines[usize::from(idx)].intersects_bbox(bbox))
                .collect();

            if intersecting.is_empty() {
                return true;
            }

            for &idx in &intersecting {
                let line = &self.level.lines[usize::from(idx)];
                let to_start = pt(start.x - line.a.x, start.y - line.a.y);
                if geo2d::dot(line.normal, to_start) <= 0.0 {
                    continue;
                }
                push_corners_away_from_line(bbox, line);
            }
        }
        false
    }

    /// Find an authorized position by trying concentric rings of 16
    /// directions at increasing radii.
    pub fn find_authorized_position_approx(
        &self,
        bbox: &mut BBox2D,
        layer: u16,
        radius: f32,
        step: f32,
    ) -> bool {
        let initial = *bbox;
        if self.find_authorized_position(bbox, layer) {
            return true;
        }

        let center_initial = initial.center();
        let mut radius_try = step;
        // Use a `loop`/`break` so the first ring (at radius = step)
        // always runs even when `step >= radius` — the original is a
        // do/while that always fires once. The original also never
        // advanced its radius variable, which would infinite-loop if
        // the first ring ever failed for all 16 directions; the
        // `radius_try += step` advance here makes the search terminate
        // instead.
        loop {
            for dir_idx in 0..16u32 {
                let angle = dir_idx as f32 * std::f32::consts::FRAC_PI_8;
                let vx = angle.sin();
                let vy = -angle.cos();
                *bbox = initial;
                bbox.translate(pt(vx * radius_try, vy * radius_try));
                if self.find_authorized_position_toward_geo(bbox, center_initial, layer) {
                    return true;
                }
            }
            if radius_try >= radius {
                break;
            }
            radius_try += step;
        }
        false
    }
}

/// Push every corner of `bbox` away from `line` when the corner is on
/// the positive-normal side (within -0.1 tolerance), translating the
/// bbox by `(dist + 1) * normal` per triggered corner. Shared by the
/// three `find_authorized_position*` variants.
///
/// Each corner is re-read between pushes so a translation triggered by
/// an earlier corner propagates to the remaining corners (typically
/// dropping their distance below the `-0.1` gate, suppressing the
/// push). Capturing corners into an array before the loop over-pushes
/// the box, so don't refactor that away.
fn push_corners_away_from_line(bbox: &mut BBox2D, line: &GridLine) {
    // Corner-read order: top-left, bottom-right, (xmax, ymin), (xmin, ymax).
    let push_if_close = |bbox: &mut BBox2D, corner: GeoPoint2D| {
        let to_corner = pt(line.a.x - corner.x, line.a.y - corner.y);
        let dist = geo2d::dot(line.normal, to_corner);
        if dist > -0.1 {
            let push = pt((dist + 1.0) * line.normal.x, (dist + 1.0) * line.normal.y);
            bbox.translate(push);
        }
    };

    push_if_close(bbox, bbox.top_left());
    push_if_close(bbox, bbox.bottom_right());
    push_if_close(bbox, pt(bbox.x_max(), bbox.y_min()));
    push_if_close(bbox, pt(bbox.x_min(), bbox.y_max()));
}

// ─── ThickMoveCorridor ───────────────────────────────────────────

/// The result of building a thick movement corridor.
///
/// Contains the two parallel side segments, bounding box, and
/// pre-computed edge vectors for point-in-corridor testing.
#[derive(Debug, Clone)]
pub struct ThickMoveCorridor {
    /// Right/top side segment.
    pub seg1: geo::Line<f32>,
    /// Left/bottom side segment (direction reversed from seg1).
    pub seg2: geo::Line<f32>,
    /// Bounding box enclosing the entire corridor.
    pub bbox: BBox2D,
    /// Direction vector of seg1.
    pub vec1: Vec2D,
    /// Direction vector of seg2.
    pub vec2: Vec2D,
    /// Closing edge vector at the destination end.
    pub vec3: Vec2D,
    /// Closing edge vector at the source end.
    pub vec4: Vec2D,
}

impl ThickMoveCorridor {
    /// Test if a point lies inside the corridor using the four-determinant test.
    /// The point is inside iff the determinant of each edge vector with
    /// `(point - edge_start)` is positive for all four edges.
    #[inline]
    pub fn point_inside(&self, p: GeoPoint2D) -> bool {
        let d1 = geo2d::cross(
            self.vec1,
            pt(p.x - self.seg1.start.x, p.y - self.seg1.start.y),
        );
        let d2 = geo2d::cross(
            self.vec2,
            pt(p.x - self.seg2.start.x, p.y - self.seg2.start.y),
        );
        let d3 = geo2d::cross(self.vec3, pt(p.x - self.seg1.end.x, p.y - self.seg1.end.y));
        let d4 = geo2d::cross(self.vec4, pt(p.x - self.seg2.end.x, p.y - self.seg2.end.y));
        d1 > 0.0 && d2 > 0.0 && d3 > 0.0 && d4 > 0.0
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_grid_with_line() -> FastFindGrid {
        let mut grid = FastFindGrid::new();
        grid.size_map(4, 4); // 4x4 cells = 256x256 pixels
        grid.allocate_layers(1); // 1 conventional layer

        // Add a horizontal motion line across the middle
        let line = GridLine::new(MapPoint::new(0.0, 128.0), MapPoint::new(256.0, 128.0), true);
        grid.add_line(line, 0);
        grid
    }

    #[test]
    fn test_grid_sizing() {
        let mut grid = FastFindGrid::new();
        grid.size_map(10, 8);
        assert_eq!(grid.level.grid_width, 10);
        assert_eq!(grid.level.grid_height, 12); // 8 + 4
        assert!(grid.level.map_bbox.is_somewhere());
    }

    #[test]
    fn test_block_index() {
        let mut grid = FastFindGrid::new();
        grid.size_map(4, 4);
        grid.allocate_layers(1);

        // Point (65, 65) should be in cell (1, 1) on layer 0
        let idx = grid.get_block_index(MapPoint::new(65.0, 65.0), 0);
        assert_eq!(idx, 1 + 4); // x=1, y=1, width=4

        // Same point on layer 1
        let idx_l1 = grid.get_block_index(MapPoint::new(65.0, 65.0), 1);
        assert_eq!(idx_l1, 1 + 4 * (1 + 8)); // height is 4+4=8
    }

    #[test]
    fn test_line_insertion_and_query() {
        let grid = make_grid_with_line();

        // Query a bbox that should contain the line
        let bbox = BBox2D::from_coords(0.0, 100.0, 256.0, 150.0);
        let lines = grid.get_active_motion_line_indices(0, &bbox);
        assert!(!lines.is_empty(), "should find the motion line");

        // Query a bbox that should NOT contain the line
        let bbox_miss = BBox2D::from_coords(0.0, 0.0, 256.0, 50.0);
        let lines_miss = grid.get_active_motion_line_indices(0, &bbox_miss);
        assert!(lines_miss.is_empty(), "should not find lines above");
    }

    #[test]
    fn test_motion_normal_matches_original_area_flag() {
        let mut area_line = GridLine::new(MapPoint::new(0.0, 0.0), MapPoint::new(10.0, 0.0), true);
        area_line.initialize_motion_normal(true);
        assert_eq!(area_line.normal, pt(0.0, 1.0));

        let mut obstacle_line =
            GridLine::new(MapPoint::new(0.0, 0.0), MapPoint::new(10.0, 0.0), true);
        obstacle_line.initialize_motion_normal(false);
        assert_eq!(obstacle_line.normal, pt(0.0, -1.0));
    }

    #[test]
    fn test_is_position_authorized() {
        let grid = make_grid_with_line();

        // A box above the line — should be OK
        let bbox_ok = BBox2D::from_coords(50.0, 50.0, 70.0, 70.0);
        assert!(grid.is_position_authorized(&bbox_ok, 0));

        // A box crossing the line — should fail
        let bbox_cross = BBox2D::from_coords(50.0, 120.0, 70.0, 140.0);
        assert!(!grid.is_position_authorized(&bbox_cross, 0));
    }

    #[test]
    fn test_is_reachable_thick() {
        let grid = make_grid_with_line();
        let hd = pt(5.0, 5.0);

        // Movement entirely above the line — reachable
        assert!(grid.is_reachable_thick(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(200.0, 50.0),
            0,
            hd
        ));

        // Movement crossing the line — not reachable
        assert!(!grid.is_reachable_thick(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(50.0, 200.0),
            0,
            hd
        ));

        // Same point — trivially reachable
        assert!(grid.is_reachable_thick(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(50.0, 50.0),
            0,
            hd
        ));
    }

    #[test]
    fn test_thick_move_corridor_construction() {
        let hd = pt(10.0, 10.0);

        // Right + down
        let corridor =
            FastFindGrid::build_thick_move_corridor(pt(100.0, 100.0), pt(200.0, 200.0), hd)
                .unwrap();
        assert!(corridor.bbox.is_somewhere());

        // A point clearly inside the corridor
        assert!(corridor.point_inside(pt(150.0, 150.0)));

        // A point far outside
        assert!(!corridor.point_inside(pt(0.0, 0.0)));

        // Zero movement
        assert!(
            FastFindGrid::build_thick_move_corridor(pt(100.0, 100.0), pt(100.0, 100.0), hd)
                .is_none()
        );
    }

    #[test]
    fn test_find_authorized_position() {
        let grid = make_grid_with_line();

        // A box straddling the line from below — should be pushed away
        let mut bbox = BBox2D::from_coords(120.0, 125.0, 140.0, 135.0);
        let found = grid.find_authorized_position(&mut bbox, 0);
        assert!(found, "should find an authorized position");
        // The pushed box should no longer intersect the line
        assert!(grid.is_position_authorized(&bbox, 0));
    }

    #[test]
    fn test_is_straight_movement_authorized() {
        let grid = make_grid_with_line();
        // 0-centered move box (half-size 5x5)
        let move_box = BBox2D::from_coords(-5.0, -5.0, 5.0, 5.0);

        // Movement entirely above the line — authorized
        assert!(grid.is_straight_movement_authorized(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(200.0, 50.0),
            0,
            &move_box
        ));

        // Movement crossing the line — not authorized
        assert!(!grid.is_straight_movement_authorized(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(50.0, 200.0),
            0,
            &move_box
        ));

        // Destination on the line — not authorized (position check fails)
        assert!(!grid.is_straight_movement_authorized(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(50.0, 128.0),
            0,
            &move_box
        ));
    }

    #[test]
    fn test_is_reachable_thick_mobile() {
        let grid = make_grid_with_line();
        let hd = pt(5.0, 5.0);

        // With no mobile repulsive lines the call collapses to
        // `is_reachable_thick`.
        assert!(grid.is_reachable_thick_mobile(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(200.0, 50.0),
            0,
            hd,
            &[]
        ));
        assert!(!grid.is_reachable_thick_mobile(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(50.0, 200.0),
            0,
            hd,
            &[]
        ));

        // Mobile repulsive line added: a vertical line at x=150 that
        // the corridor from (50,50)→(200,50) must cross.
        let mobile = GridLine::new(
            MapPoint::new(150.0, 30.0),
            MapPoint::new(150.0, 70.0),
            false,
        );
        assert!(!grid.is_reachable_thick_mobile(
            MapPoint::new(50.0, 50.0),
            MapPoint::new(200.0, 50.0),
            0,
            hd,
            std::slice::from_ref(&mobile)
        ));
    }

    #[test]
    fn test_find_authorized_position_toward() {
        let grid = make_grid_with_line();

        // Box straddling the line, push toward a click point below (on the
        // normal side — the line's normal points +Y for a left-to-right line)
        let mut bbox = BBox2D::from_coords(120.0, 125.0, 140.0, 135.0);
        let found = grid.find_authorized_position_toward(&mut bbox, MapPoint::new(130.0, 200.0), 0);
        assert!(found, "should find position toward click");
        assert!(grid.is_position_authorized(&bbox, 0));
        // Box should have been pushed below the line
        assert!(bbox.y_min() > 128.0);
    }

    #[test]
    fn test_find_authorized_position_straight() {
        let grid = make_grid_with_line();

        // Box straddling the line, push along segment from a start point below
        let mut bbox = BBox2D::from_coords(120.0, 125.0, 140.0, 135.0);
        let found =
            grid.find_authorized_position_straight(&mut bbox, MapPoint::new(130.0, 200.0), 0);
        assert!(found, "should find straight authorized position");
        assert!(grid.is_position_authorized(&bbox, 0));
    }

    #[test]
    fn test_find_authorized_position_approx() {
        let grid = make_grid_with_line();

        // Box right on the line — approx search should find a nearby clear position
        let mut bbox = BBox2D::from_coords(120.0, 125.0, 140.0, 135.0);
        let found = grid.find_authorized_position_approx(&mut bbox, 0, 100.0, 20.0);
        assert!(found, "should find approx authorized position");
        assert!(grid.is_position_authorized(&bbox, 0));
    }

    // ── Helpers for the new-feature tests ──

    fn make_empty_grid(layers: u16) -> FastFindGrid {
        let mut grid = FastFindGrid::new();
        grid.size_map(8, 8);
        grid.allocate_layers(layers);
        grid
    }

    fn square_sector(
        min: GeoPoint2D,
        max: GeoPoint2D,
        sector_type: crate::sector::SectorType,
        layer: u16,
        sector_number: i16,
    ) -> GridSector {
        let pts = vec![
            pt(min.x, min.y),
            pt(max.x, min.y),
            pt(max.x, max.y),
            pt(min.x, max.y),
        ];
        let mut bbox = BBox2D::new();
        for &p in &pts {
            bbox.expand_point(p);
        }
        GridSector {
            points: pts.iter().copied().map(MapPoint::from_geo).collect(),
            bounding_box: bbox,
            sector_type,
            layer,
            sector_number: crate::sector::SectorNumber::new(sector_number),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        }
    }

    fn square_projection_obstacle(
        min: GeoPoint2D,
        max: GeoPoint2D,
        layer: u16,
        sector: u16,
    ) -> crate::sight_obstacle::SightObstacle {
        use crate::sight_obstacle::{
            ObstaclePoint, SIGHTOBSTACLE_PROJECTION_AREA, SIGHTOBSTACLE_SOLID, SightObstacle,
        };

        let mut obs = SightObstacle::new(0, SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_PROJECTION_AREA);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: min.x,
                y: min.y,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            ObstaclePoint {
                x: max.x,
                y: min.y,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            ObstaclePoint {
                x: max.x,
                y: max.y,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            ObstaclePoint {
                x: min.x,
                y: max.y,
                z_bottom: 0.0,
                z_top: 10.0,
            },
        ];
        obs.layer = layer;
        obs.sector = sector;
        obs.rebuild_geometry();
        obs
    }

    #[test]
    fn projectile_landing_resolves_projection_area_layer_and_motion_sector() {
        use crate::sector::SectorType;

        let mut grid = make_empty_grid(2);
        grid.add_sector(
            square_sector(
                pt(0.0, 0.0),
                pt(128.0, 128.0),
                SectorType::MOTION | SectorType::AREA,
                1,
                12,
            ),
            1,
        );
        let obstacles = [square_projection_obstacle(
            pt(0.0, 0.0),
            pt(128.0, 128.0),
            1,
            12,
        )];

        let resolution = grid.resolve_projectile_landing(
            MapPoint::new(64.0, 54.0),
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
        );

        assert_eq!(resolution.layer, 1);
        assert_eq!(resolution.obstacle_index.map(u16::from), Some(0));
        assert_eq!(resolution.sector.map(u16::from), Some(12));
        assert!(!resolution.blocked_by_motion_obstacle);
    }

    #[test]
    fn projectile_landing_prefers_exact_projection_obstacle_identity() {
        use crate::position_interface::ObstacleHandle;
        use crate::sector::SectorType;

        let mut grid = make_empty_grid(3);
        grid.add_sector(
            square_sector(
                pt(0.0, 0.0),
                pt(128.0, 128.0),
                SectorType::MOTION | SectorType::AREA,
                2,
                22,
            ),
            2,
        );
        let obstacles = [
            square_projection_obstacle(pt(0.0, 0.0), pt(128.0, 128.0), 1, 11),
            square_projection_obstacle(pt(0.0, 0.0), pt(128.0, 128.0), 2, 22),
        ];

        let resolution = grid.resolve_projectile_landing_with_obstacle(
            MapPoint::new(64.0, 54.0),
            ObstacleHandle::new(1),
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
        );

        assert_eq!(resolution.obstacle_index.map(u16::from), Some(1));
        assert_eq!(resolution.layer, 2);
        assert_eq!(resolution.sector.map(u16::from), Some(22));
    }

    #[test]
    fn projectile_landing_motion_obstacle_clears_sector() {
        use crate::sector::SectorType;

        let mut grid = make_empty_grid(1);
        grid.add_sector(
            square_sector(
                pt(0.0, 0.0),
                pt(128.0, 128.0),
                SectorType::MOTION | SectorType::AREA,
                0,
                5,
            ),
            0,
        );
        grid.add_sector(
            square_sector(pt(48.0, 48.0), pt(80.0, 80.0), SectorType::MOTION, 0, 6),
            0,
        );

        let resolution = grid.resolve_projectile_landing(
            MapPoint::new(64.0, 64.0),
            crate::sight_obstacle::ObstacleList::empty(),
        );

        assert_eq!(resolution.layer, 0);
        assert_eq!(resolution.sector, None);
        assert!(resolution.blocked_by_motion_obstacle);
    }

    #[test]
    fn test_is_valid_for_move_accepts_jump() {
        use crate::sector::SectorType;
        let mut grid = make_empty_grid(1);
        // Motion area under the jump overlay.
        let area = square_sector(
            pt(32.0, 32.0),
            pt(96.0, 96.0),
            SectorType::MOUSE | SectorType::MOTION | SectorType::AREA,
            0,
            1,
        );
        grid.add_sector(area, 0);
        let jump = square_sector(
            pt(40.0, 40.0),
            pt(80.0, 80.0),
            SectorType::MOUSE | SectorType::JUMP,
            0,
            2,
        );
        grid.add_sector(jump, 0);

        let res = grid.get_sector_screen(MapPoint::new(60.0, 60.0), MapPoint::new(60.0, 60.0));
        assert!(res.is_valid());
        assert!(res.is_valid_for_move(&grid));
    }

    #[test]
    fn test_get_sector_jump_third_pass() {
        use crate::sector::SectorType;
        let mut grid = make_empty_grid(1);

        // Motion area covering (0..128, 0..128).
        grid.add_sector(
            square_sector(
                pt(0.0, 0.0),
                pt(128.0, 128.0),
                SectorType::MOUSE | SectorType::MOTION | SectorType::AREA,
                0,
                10,
            ),
            0,
        );

        // Two jump lines the grid can reference from the jump sectors.
        let jl_a = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(40.0, 60.0),
            crate::coordinates::map_pt(80.0, 60.0),
            0.0,
            0.0,
        );
        let jl_b = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(40.0, 90.0),
            crate::coordinates::map_pt(80.0, 90.0),
            0.0,
            0.0,
        );
        grid.level_mut().jump_lines.push(jl_a); // idx 0, mid = (60, 60)
        grid.level_mut().jump_lines.push(jl_b); // idx 1, mid = (60, 90)

        // Two overlapping jump sectors covering the same click point.
        let mut jump_a = square_sector(
            pt(32.0, 32.0),
            pt(96.0, 96.0),
            SectorType::MOUSE | SectorType::JUMP,
            0,
            20,
        );
        jump_a
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(0).unwrap());
        grid.add_sector(jump_a, 0);

        let mut jump_b = square_sector(
            pt(32.0, 32.0),
            pt(96.0, 96.0),
            SectorType::MOUSE | SectorType::JUMP,
            0,
            21,
        );
        jump_b
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(1).unwrap());
        grid.add_sector(jump_b, 0);

        // Reference close to jump_a's first line midpoint (60,60).
        let hit_a = grid.get_sector(MapPoint::new(60.0, 70.0), MapPoint::new(55.0, 55.0), 0);
        match hit_a {
            SectorHit::Found { sector_number, .. } => assert_eq!(sector_number, 20),
            _ => panic!("expected jump_a to win"),
        }

        // Reference close to jump_b's first line midpoint (60,90).
        let hit_b = grid.get_sector(MapPoint::new(60.0, 70.0), MapPoint::new(55.0, 95.0), 0);
        match hit_b {
            SectorHit::Found { sector_number, .. } => assert_eq!(sector_number, 21),
            _ => panic!("expected jump_b to win"),
        }
    }

    #[test]
    fn test_get_sector_screen_invalid_has_no_sector() {
        let grid = make_empty_grid(1);

        let res = grid.get_sector_screen(MapPoint::new(60.0, 60.0), MapPoint::new(60.0, 60.0));

        assert!(!res.is_valid());
        assert_eq!(res.sector_idx, None);
        assert_eq!(res.sector, None);
    }

    #[test]
    fn test_add_obstacle_index_populates_block_and_layer() {
        let mut grid = make_empty_grid(1);
        let bbox = BBox2D::from_coords(64.0, 64.0, 127.0, 127.0);
        let idx_42 = crate::sight_obstacle::SightObstacleIndex::new(42).unwrap();
        grid.add_obstacle_index(idx_42, 0, &bbox);

        // The per-layer list must contain the obstacle index.
        assert_eq!(grid.level.layers[0].obstacle_indices, vec![idx_42]);

        // A query in the obstacle's cell returns the index.
        let hit = grid.get_obstacle_indices(0, &bbox);
        assert!(hit.contains(&idx_42));

        // A query in an unrelated cell does not.
        let miss_bbox = BBox2D::from_coords(256.0, 256.0, 300.0, 300.0);
        let miss = grid.get_obstacle_indices(0, &miss_bbox);
        assert!(miss.is_empty());
    }

    #[test]
    fn test_is_reachable_impact_3d_blocks_over_top_plane() {
        use crate::sight_obstacle::{
            ObstaclePoint, SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID, SightObstacle,
        };
        let grid = make_empty_grid(1);

        // A 10-tall wall between x=100 and x=150.
        let mut obs = SightObstacle::new(0, SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: 40.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 150.0,
                y: 40.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 150.0,
                y: 80.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 80.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();
        // Top plane at z=10, bottom plane at z=0 (on ground).
        obs.top_plane_points = [[0.0, 0.0, 10.0], [1.0, 0.0, 10.0], [0.0, 1.0, 10.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let obstacles = vec![obs];

        let origin = crate::coordinates::WorldPoint3D {
            x: 50.0,
            y: 60.0,
            z: 5.0,
        };
        let dest = crate::coordinates::WorldPoint3D {
            x: 200.0,
            y: 60.0,
            z: 5.0,
        };
        let mut impact = None;
        let clear = grid.is_reachable_impact_3d(
            origin,
            dest,
            0,
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
            &mut impact,
        );
        assert!(!clear, "wall should block the 3D ray at eye level");
        assert!(impact.is_some());

        // A ray well above the top plane (z = 20) clears the wall.
        let origin_high = crate::coordinates::WorldPoint3D {
            x: 50.0,
            y: 60.0,
            z: 20.0,
        };
        let dest_high = crate::coordinates::WorldPoint3D {
            x: 200.0,
            y: 60.0,
            z: 20.0,
        };
        let mut impact_high = None;
        let clear_high = grid.is_reachable_impact_3d(
            origin_high,
            dest_high,
            0,
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
            &mut impact_high,
        );
        assert!(clear_high, "ray above the top plane should pass");
        assert!(impact_high.is_none());
    }
}
