//! Sight obstacles -- 3D obstacles that block line of sight for AI detection.
//!
//! A sight obstacle is a convex polygon (ground projection) with per-vertex
//! top/bottom Z heights, plus 3D planes describing its top and bottom faces.
//! The engine uses these to cull AI vision rays.

use serde::{Deserialize, Serialize};

use crate::coordinates::{GroundBBox, GroundPoint, MapBBox, MapPoint};
use crate::geo2d::{self, Polygon2D, pt, segment};

// ---------------------------------------------------------------------------
// SightObstacleIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `EngineInner::sight_obstacles` (the flat static + dynamic
/// view exposed by [`ObstacleList`]).  Wraps [`nonmax::NonMaxU32`] so
/// `Option<SightObstacleIndex>` is 4 bytes via the niche.
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
pub struct SightObstacleIndex(pub nonmax::NonMaxU32);

impl SightObstacleIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<SightObstacleIndex> for u32 {
    #[inline]
    fn from(i: SightObstacleIndex) -> u32 {
        i.0.get()
    }
}
impl From<SightObstacleIndex> for usize {
    #[inline]
    fn from(i: SightObstacleIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for SightObstacleIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

// ─── Two-part obstacle list (static + dynamic) ────────────────────

/// Borrowed view over the level's static sight obstacles plus any
/// per-frame dynamic obstacles (currently just shields). Replaces the
/// flat `&[SightObstacle]` parameter that pre-LevelGrid code used to
/// pass around.
///
/// Static obstacles live in `LevelAssets::static_sight_obstacles`
/// (Arc-shared so `EngineInner::clone` is cheap); dynamic obstacles live in
/// `EngineInner::dynamic_sight_obstacles` and are rebuilt each tick by
/// `update_shield_obstacles`. The "global obstacle index" used by
/// patches and per-actor `obstacle_index` lookups continues to be a
/// flat 0..N indexing — entries 0..static_len() come from the static
/// slice, entries static_len().. come from the dynamic slice.
#[derive(Debug, Clone, Copy)]
pub struct ObstacleList<'a> {
    pub static_obstacles: &'a [SightObstacle],
    pub dynamic_obstacles: &'a [SightObstacle],
    /// Per-static-obstacle runtime active flag (parallel to
    /// `static_obstacles`). Dynamic obstacles are implicitly active.
    pub static_active: &'a [bool],
}

impl<'a> ObstacleList<'a> {
    pub fn empty() -> Self {
        Self {
            static_obstacles: &[],
            dynamic_obstacles: &[],
            static_active: &[],
        }
    }

    /// Build an ObstacleList over a static slice, treating every entry
    /// as active. Convenience for tests / call sites that don't carry
    /// the parallel active-flag array. The returned view's lifetime
    /// covers `obstacles` for both the geometry slice and a per-call
    /// implicit "all true" active-flag slice.
    pub fn from_slice_all_active(obstacles: &'a [SightObstacle]) -> Self {
        // SAFETY-ish: `static_active` length == `obstacles.len()` and
        // every entry is `true`. We can't materialize a temporary
        // `Vec<bool>` here without leaking, so callers that need the
        // active flag must pre-build it. For test code this constructor
        // pairs with `is_active(idx)` returning true based on the
        // "fallback" branch when the slice doesn't reach `idx` —
        // see [`Self::is_active`].
        Self {
            static_obstacles: obstacles,
            dynamic_obstacles: &[],
            static_active: &[],
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.static_obstacles.len() + self.dynamic_obstacles.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<&'a SightObstacle> {
        let s = self.static_obstacles.len();
        if idx < s {
            self.static_obstacles.get(idx)
        } else {
            self.dynamic_obstacles.get(idx - s)
        }
    }

    /// Whether the obstacle at `idx` is currently active.
    /// When `static_active` is shorter than `static_obstacles` (e.g.
    /// in unit tests that build an `ObstacleList` with `&[]`), missing
    /// entries default to `true`. The engine path always populates the
    /// flag array length-paired with the obstacle list so the default
    /// only kicks in for test convenience.
    #[inline]
    pub fn is_active(&self, idx: usize) -> bool {
        let s = self.static_obstacles.len();
        if idx < s {
            self.static_active.get(idx).copied().unwrap_or(true)
        } else {
            // Dynamic obstacles (shields) are always active.
            idx - s < self.dynamic_obstacles.len()
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a SightObstacle> + Clone + 'a {
        self.static_obstacles
            .iter()
            .chain(self.dynamic_obstacles.iter())
    }

    /// `(idx, &obstacle)` pairs in flat-index order.
    pub fn iter_indexed(&self) -> impl Iterator<Item = (u32, &'a SightObstacle)> + Clone + 'a {
        let s = self.static_obstacles.len();
        self.static_obstacles
            .iter()
            .enumerate()
            .map(|(i, o)| (i as u32, o))
            .chain(
                self.dynamic_obstacles
                    .iter()
                    .enumerate()
                    .map(move |(i, o)| ((s + i) as u32, o)),
            )
    }
}

/// Per-tick `Arc`-shareable snapshot of the engine's static + dynamic
/// sight obstacles plus the static-active flag array.  Built once at
/// the top of each AI dispatch pass by
/// [`crate::engine::EngineInner::build_sim_scratch`] and
/// embedded into every [`crate::ai::AiContext`] so AI helpers can run
/// `ai_vision::los_clear` without re-borrowing the engine.
///
/// Same `Arc<HashMap>` pattern used for
/// [`crate::ai_entity_view::SharedAiEntityViews`]: cloning is a single
/// atomic increment per `AiContext`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SharedSightObstacles {
    pub static_obstacles: std::sync::Arc<Vec<SightObstacle>>,
    pub dynamic_obstacles: std::sync::Arc<Vec<SightObstacle>>,
    pub static_active: std::sync::Arc<Vec<bool>>,
}

impl Default for SharedSightObstacles {
    fn default() -> Self {
        Self {
            static_obstacles: std::sync::Arc::new(Vec::new()),
            dynamic_obstacles: std::sync::Arc::new(Vec::new()),
            static_active: std::sync::Arc::new(Vec::new()),
        }
    }
}

impl SharedSightObstacles {
    /// Borrowed [`ObstacleList`] view over the snapshot — the shape that
    /// `ai_vision::los_clear` and the per-obstacle visibility helpers
    /// already accept.
    pub fn list(&self) -> ObstacleList<'_> {
        ObstacleList {
            static_obstacles: &self.static_obstacles,
            dynamic_obstacles: &self.dynamic_obstacles,
            static_active: &self.static_active,
        }
    }
}

// ---- Obstacle type flags ----

/// Bitflag constants for `SightObstacle::obstacle_type`. Stored as a
/// single integer used as a bitfield.
pub const SIGHTOBSTACLE_SOLID: u32 = 1;
pub const SIGHTOBSTACLE_OPAQUE: u32 = 2;
pub const SIGHTOBSTACLE_PROJECTION_AREA: u32 = 4;
pub const SIGHTOBSTACLE_MOUSE: u32 = 8;
pub const SIGHTOBSTACLE_SHIELD: u32 = 16;
pub const SIGHTOBSTACLE_SHOW_SHADOW_POLYGON: u32 = 32;

/// One authoritative opaque reachability call observed while replay parity
/// capture is active. The game uses only the ordered endpoints and boolean
/// result; obstacle/cache diagnostics remain recorder-side explanation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParityVisibilityQuery {
    pub origin: [f32; 3],
    pub destination: [f32; 3],
    pub result: bool,
}

thread_local! {
    static PARITY_VISIBILITY_CAPTURE: std::cell::RefCell<Option<Vec<ParityVisibilityQuery>>> =
        const { std::cell::RefCell::new(None) };
}

/// Begin ordered visibility-call capture on the simulation thread.
///
/// This is deliberately opt-in so ordinary gameplay does not allocate or
/// synchronize on every line-of-sight query.
pub fn begin_parity_visibility_capture() {
    PARITY_VISIBILITY_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        assert!(
            capture.is_none(),
            "parity visibility capture was begun twice without being drained"
        );
        *capture = Some(Vec::new());
    });
}

/// Finish visibility-call capture and return calls in execution order.
pub fn take_parity_visibility_capture() -> Vec<ParityVisibilityQuery> {
    PARITY_VISIBILITY_CAPTURE.with(|capture| {
        capture
            .borrow_mut()
            .take()
            .expect("parity visibility capture was drained without being started")
    })
}

fn record_parity_visibility_query(
    origin: [f32; 3],
    destination: [f32; 3],
    type_mask: u32,
    result: bool,
) {
    if type_mask != SIGHTOBSTACLE_OPAQUE {
        return;
    }
    PARITY_VISIBILITY_CAPTURE.with(|capture| {
        if let Some(queries) = capture.borrow_mut().as_mut() {
            queries.push(ParityVisibilityQuery {
                origin,
                destination,
                result,
            });
        }
    });
}

// ---- ObstaclePoint ----

/// A vertex of the obstacle with ground (x, y) and height range (z_bottom..z_top).
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ObstaclePoint {
    pub x: f32,
    pub y: f32,
    pub z_top: f32,
    pub z_bottom: f32,
}

impl ObstaclePoint {
    pub fn ground_point(&self) -> GroundPoint {
        GroundPoint::new(self.x, self.y)
    }
}

/// Ground-plane obstacle polygon, matching C++ `RHSightObstacle::mPolygon`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct GroundPolygon(Polygon2D<f32>);

impl GroundPolygon {
    pub fn empty() -> Self {
        Self(Polygon2D::new(geo::LineString::new(vec![]), vec![]))
    }

    pub fn from_ground_points(points: impl IntoIterator<Item = GroundPoint>) -> Self {
        let coords: Vec<geo::Coord<f32>> = points.into_iter().map(GroundPoint::to_geo).collect();
        Self(Polygon2D::new(geo::LineString::from(coords), vec![]))
    }

    pub fn as_geo(&self) -> &Polygon2D<f32> {
        &self.0
    }
}

/// Projected obstacle polygon with vertices `(x, y - z_top)`.
///
/// This is separate from [`GroundPolygon`] because the original C++ keeps
/// `mPolygon` in ground coordinates while `mBoxScreen` is projected.  Rust
/// also stores a projected polygon for projection-area clipping/lookup.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ProjectionPolygon(Polygon2D<f32>);

impl ProjectionPolygon {
    pub fn empty() -> Self {
        Self(Polygon2D::new(geo::LineString::new(vec![]), vec![]))
    }

    pub fn from_projected_points(points: impl IntoIterator<Item = MapPoint>) -> Self {
        let coords: Vec<geo::Coord<f32>> = points.into_iter().map(MapPoint::to_geo).collect();
        Self(Polygon2D::new(geo::LineString::from(coords), vec![]))
    }

    pub fn as_geo(&self) -> &Polygon2D<f32> {
        &self.0
    }
}

// ---- SightObstacle ----

/// A 3D sight obstacle that blocks line of sight for AI detection.
///
/// The obstacle is defined by a set of vertices (`obstacle_points`) whose
/// ground-plane projection forms `polygon`.  Each vertex carries independent
/// top / bottom Z heights; two 3D planes (`top_plane`, `bottom_plane`)
/// describe the cap surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SightObstacle {
    /// Unique ID (monotonically increasing, assigned at construction).
    pub id: u32,

    /// Bitfield of `SIGHTOBSTACLE_*` flags.
    pub obstacle_type: u32,

    // The runtime `active` toggle (set by patches) lives in
    // [`EngineInner::static_sight_obstacle_active`] for static obstacles
    // (parallel to `LevelAssets::static_sight_obstacles`). Dynamic
    // obstacles (shields) are implicitly always active.
    /// 3D axis-aligned bounding box (min/max corners as `[x, y, z]`).
    pub box_3d_min: [f32; 3],
    pub box_3d_max: [f32; 3],

    /// 2D ground-plane bounding box.
    pub box_ground: GroundBBox,

    /// 2D projected bounding box (Y shifted by Z for isometric projection).
    pub box_projection: MapBBox,

    /// Per-vertex obstacle data (x, y, z_top, z_bottom).
    pub obstacle_points: Vec<ObstaclePoint>,

    /// Ground-plane polygon (CCW winding, convex).
    pub polygon: GroundPolygon,

    /// Projected polygon — vertices `(x, y - z_top)`.  Used to
    /// discriminate candidate projection-area obstacles by
    /// point-in-polygon at the position's projected map coordinates.
    /// Only meaningful when `is_projection_area()` is set; for
    /// ground-flat obstacles it coincides with `polygon`.
    pub polygon_projection: ProjectionPolygon,

    /// Top plane defined by three points `[origin, p1, p2]` (each `[x,y,z]`).
    /// Stored as raw triples so we don't depend on sb3d serde.
    pub top_plane_points: [[f32; 3]; 3],

    /// Bottom plane defined by three points.
    pub bottom_plane_points: [[f32; 3]; 3],

    /// Whether the obstacle sits on the ground (all z_bottom == 0).
    pub on_ground: bool,

    /// Layer index (`u16::MAX` if not a projection area).
    pub layer: u16,

    /// Sector index (`u16::MAX` if not a projection area).
    pub sector: u16,

    /// Vertical bounce factor for projectile reflection.
    pub bounce_vertical: f32,

    /// Horizontal bounce factor for projectile reflection.
    pub bounce_horizontal: f32,

    /// Material type index (for footstep / impact sounds).
    pub material: u8,

    /// Per-obstacle material sub-sectors (heterogeneous surface — e.g.
    /// a stone inlay on a wooden platform).  Populated at level load
    /// from `RawSightObstacle::material_indices` references into the
    /// global material-sector list.
    ///
    /// The obstacle holds clones of the polygons it covers in the
    /// global material-sector list. Used by projectile material /
    /// water-hole detection to find sub-sectors carved into the
    /// obstacle. Also see
    /// [`crate::material_sectors::MaterialSectors::material_at_with_obstacle`].
    /// Empty for obstacles with no material-sector references in the
    /// proto stream and for runtime-built obstacles (shields, ad-hoc
    /// walls).
    pub material_sectors: Vec<crate::material_sectors::MaterialSector>,

    /// Probability (0..255) that stepping on this surface triggers a sound.
    pub sound_probability: u8,

    // ---- Runtime sight-check hints (not serialized in saves) ----
    /// Whether useful for downward line-of-sight checks.
    pub useful_for_downward: bool,
    /// Whether useful for upward line-of-sight checks.
    pub useful_for_upward: bool,
    /// Whether the viewer is above the obstacle.
    pub viewer_above: bool,
    /// Whether the viewer is below the obstacle.
    pub viewer_below: bool,
}

impl SightObstacle {
    /// Create a new obstacle with the given type flags and auto-assigned ID.
    pub fn new(id: u32, obstacle_type: u32) -> Self {
        Self {
            id,
            obstacle_type,
            box_3d_min: [0.0; 3],
            box_3d_max: [0.0; 3],
            box_ground: GroundBBox::new(),
            box_projection: MapBBox::new(),
            obstacle_points: Vec::new(),
            polygon: GroundPolygon::empty(),
            polygon_projection: ProjectionPolygon::empty(),
            top_plane_points: [[0.0; 3]; 3],
            bottom_plane_points: [[0.0; 3]; 3],
            on_ground: true,
            layer: u16::MAX,
            sector: u16::MAX,
            bounce_vertical: 1.0,
            bounce_horizontal: 1.0,
            material: 0,
            material_sectors: Vec::new(),
            sound_probability: 0,
            useful_for_downward: false,
            useful_for_upward: false,
            viewer_above: false,
            viewer_below: false,
        }
    }

    /// Default constructor — SOLID|OPAQUE.
    pub fn new_default(id: u32) -> Self {
        Self::new(id, SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE)
    }

    // ---- Type flag queries ----

    #[inline]
    pub fn is_solid(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_SOLID != 0
    }

    /// C++ `RHSightObstacle::IsOfType`: every bit requested by `required`
    /// must be present. This is deliberately not an any-of test. In
    /// particular, `SOLID | OPAQUE` excludes solid-only shadow geometry.
    #[inline]
    pub fn is_of_type(&self, required: u32) -> bool {
        self.obstacle_type & required == required
    }

    #[inline]
    pub fn is_opaque(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_OPAQUE != 0
    }

    #[inline]
    pub fn is_projection_area(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_PROJECTION_AREA != 0
    }

    #[inline]
    pub fn is_mouse(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_MOUSE != 0
    }

    #[inline]
    pub fn is_shield(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_SHIELD != 0
    }

    #[inline]
    pub fn is_showing_shadow_polygon(&self) -> bool {
        self.obstacle_type & SIGHTOBSTACLE_SHOW_SHADOW_POLYGON != 0
    }

    // ---- Type flag setters ----

    pub fn set_flag(&mut self, flag: u32, state: bool) {
        if state {
            self.obstacle_type |= flag;
        } else {
            self.obstacle_type &= !flag;
        }
    }

    // ---- Geometry queries ----

    /// Test if a ground-plane point lies inside the obstacle's polygon.
    pub fn contains_point(&self, p: GroundPoint) -> bool {
        geo2d::polygon_contains_point(self.polygon.as_geo(), p.to_geo())
    }

    /// Test if a projected map point lies inside the obstacle's
    /// projected polygon (vertices `(x, y - z_top)`).  Used by the
    /// projection-area sector lookup.
    pub fn contains_point_projection(&self, p: MapPoint) -> bool {
        geo2d::polygon_contains_point(self.polygon_projection.as_geo(), p.to_geo())
    }

    /// Test if a sight line (segment from `from` to `to` on the ground plane)
    /// is blocked by this obstacle.
    ///
    /// Returns `true` when the obstacle is active and the segment intersects
    /// the ground-plane polygon.  The caller is responsible for filtering by
    /// bounding-box first (typically done by the fast-find grid).
    pub fn is_blocking_sight(&self, from: GroundPoint, to: GroundPoint) -> bool {
        let seg = segment(from.to_geo(), to.to_geo());
        // Quick AABB rejection before the full polygon test.
        if self.box_ground.trivially_rejects_segment(seg) {
            return false;
        }
        geo2d::segment_intersects_polygon(seg, self.polygon.as_geo())
    }

    // ---- Polygon / bounding-box construction helpers ----

    /// Rebuild the ground polygon and bounding boxes from `obstacle_points`.
    /// Call this after populating or mutating `obstacle_points`.
    pub fn rebuild_geometry(&mut self) {
        // Build polygon from ground projections.
        let coords = self
            .obstacle_points
            .iter()
            .map(|op| GroundPoint::new(op.x, op.y));

        self.polygon = GroundPolygon::from_ground_points(coords);

        // Build projected polygon (vertices `(x, y - z_top)`). Used
        // for projected point-in-polygon discrimination by the
        // projection-area sector lookup.
        let coords_screen = self
            .obstacle_points
            .iter()
            .map(|op| MapPoint::from_world_xyz(op.x, op.y, op.z_top));
        self.polygon_projection = ProjectionPolygon::from_projected_points(coords_screen);

        // Rebuild 2D ground bbox.
        self.box_ground = GroundBBox::new();
        for op in &self.obstacle_points {
            self.box_ground.expand_point(GroundPoint::new(op.x, op.y));
        }

        // Rebuild 3D bbox.
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for op in &self.obstacle_points {
            min[0] = min[0].min(op.x);
            min[1] = min[1].min(op.y);
            min[2] = min[2].min(op.z_bottom);
            max[0] = max[0].max(op.x);
            max[1] = max[1].max(op.y);
            max[2] = max[2].max(op.z_top);
        }
        self.box_3d_min = min;
        self.box_3d_max = max;

        // Rebuild projected bbox (isometric: projected_y = y - z_top).
        self.box_projection = MapBBox::new();
        for op in &self.obstacle_points {
            self.box_projection
                .expand_point(MapPoint::from_world_xyz(op.x, op.y, op.z_top));
            self.box_projection
                .expand_point(MapPoint::from_world_xyz(op.x, op.y, op.z_bottom));
        }

        // Check on_ground.
        self.on_ground = self.obstacle_points.iter().all(|op| op.z_bottom == 0.0);
    }

    /// Translate all points by a 2D vector.
    pub fn translate_2d(&mut self, dx: f32, dy: f32) {
        for op in &mut self.obstacle_points {
            op.x += dx;
            op.y += dy;
        }
        // Also shift the top/bottom planes so `compute_top_z` /
        // `compute_bottom_z` remain correct after a move.
        for p in &mut self.top_plane_points {
            p[0] += dx;
            p[1] += dy;
        }
        for p in &mut self.bottom_plane_points {
            p[0] += dx;
            p[1] += dy;
        }
        self.rebuild_geometry();
    }

    // ---- 3D plane height queries ----

    /// Compute Z height of the top plane at ground position (x, y).
    pub fn compute_top_z(&self, x: f32, y: f32) -> f32 {
        compute_plane_z(&self.top_plane_points, x, y)
    }

    /// Compute the top-plane Z at an isometrically projected map point.
    ///
    /// The input Y is `world_y - z`, so resolving the world-space plane
    /// requires the same `(1 - bz)` correction as the Original's
    /// `PositionToPoint3D`. Use [`Self::compute_top_z`] instead when Y is
    /// already a world/ground coordinate.
    pub fn compute_top_z_from_projection(&self, x: f32, projected_y: f32) -> f32 {
        crate::position_interface::PlaneZCoeffs::from_plane_points(&self.top_plane_points)
            .compute_z(x, projected_y)
    }

    /// Compute Z height of the bottom plane at ground position (x, y).
    pub fn compute_bottom_z(&self, x: f32, y: f32) -> f32 {
        compute_plane_z(&self.bottom_plane_points, x, y)
    }

    /// Top-plane origin point — the first of the three points used to
    /// define the plane.
    pub fn top_plane_origin(&self) -> [f32; 3] {
        self.top_plane_points[0]
    }

    /// Top-plane unit normal.
    ///
    /// Computed as `(p1 − p0) × (p2 − p0)` normalized.  Callers in the
    /// radius/projection path only use `|n · v|` or `(n · v)²`, so the
    /// sign is not pinned down — downstream uses that rely on a specific
    /// orientation should not assume one.
    pub fn top_plane_normal(&self) -> [f32; 3] {
        plane_unit_normal(&self.top_plane_points)
    }

    // ---- 3D ray blocking ----

    /// Test if a 3D ray from `origin` to `destination` is blocked by this obstacle.
    ///
    /// Handles both on-ground obstacles (only top plane matters) and elevated
    /// obstacles (both top and bottom planes).
    pub fn is_blocking_ray_3d(&self, origin: [f32; 3], destination: [f32; 3]) -> bool {
        let ray_seg = segment(pt(origin[0], origin[1]), pt(destination[0], destination[1]));

        // Quick AABB rejection on ground projection.
        if self.box_ground.trivially_rejects_segment(ray_seg) {
            return false;
        }

        // Compute relative heights at origin and destination vs top plane.
        let origin_rel_top = origin[2] - self.compute_top_z(origin[0], origin[1]);
        let dest_rel_top = destination[2] - self.compute_top_z(destination[0], destination[1]);
        let origin_above_top = origin_rel_top > 0.0;
        let dest_above_top = dest_rel_top > 0.0;

        if self.on_ground {
            // ── On-ground obstacle: only top plane matters ──
            // Both above top → ray passes over, no blocking.
            if origin_above_top && dest_above_top {
                return false;
            }

            // Lazy-init the ray Z equation for height interpolation.
            let ray_eq = RayZEquation::new(origin, destination);

            // Walk obstacle polygon edges: if the 2D ray crosses any edge,
            // check whether the ray is below the obstacle top at that point.
            let pts = &self.obstacle_points;
            if pts.is_empty() {
                return false;
            }
            let n = pts.len();
            let mut last_2d = pt(pts[n - 1].x, pts[n - 1].y);

            for pt_i in pts {
                let cur_2d = pt(pt_i.x, pt_i.y);
                let edge = segment(last_2d, cur_2d);

                if geo2d::segments_intersect(ray_seg, edge) {
                    // Both below top → ray definitely blocked by this wall.
                    if !origin_above_top && !dest_above_top {
                        return true;
                    }

                    // One above, one below: compute ray height at intersection.
                    if let geo2d::Intersection2D::Point(ip) =
                        geo2d::segment_intersection(ray_seg, edge)
                    {
                        let ray_z = ray_eq.z_at(GroundPoint::from_geo(ip));
                        let top_z = self.compute_top_z(ip.x, ip.y);
                        if top_z >= ray_z {
                            return true;
                        }
                    }
                }

                last_2d = cur_2d;
            }

            // Top-plane crossing: if ray goes from above to below (or vice
            // versa), check if the crossing point is inside the polygon.
            if origin_above_top != dest_above_top {
                let denom = origin_rel_top - dest_rel_top;
                if denom.abs() > 1e-9 {
                    let t = origin_rel_top / denom;
                    let ix = origin[0] + t * (destination[0] - origin[0]);
                    let iy = origin[1] + t * (destination[1] - origin[1]);
                    let ip = pt(ix, iy);
                    if self.box_ground.contains_point(GroundPoint::from_geo(ip))
                        && geo2d::polygon_contains_point(self.polygon.as_geo(), ip)
                    {
                        return true;
                    }
                }
            }
        } else {
            // ── Elevated obstacle: both top and bottom planes matter ──
            let origin_rel_bot = origin[2] - self.compute_bottom_z(origin[0], origin[1]);
            let dest_rel_bot =
                destination[2] - self.compute_bottom_z(destination[0], destination[1]);
            let origin_below_bot = origin_rel_bot < 0.0;
            let dest_below_bot = dest_rel_bot < 0.0;

            // Both above top OR both below bottom → skip.
            if (origin_above_top && dest_above_top) || (origin_below_bot && dest_below_bot) {
                return false;
            }

            let ray_eq = RayZEquation::new(origin, destination);

            let pts = &self.obstacle_points;
            if pts.is_empty() {
                return false;
            }
            let n = pts.len();
            let mut last_2d = pt(pts[n - 1].x, pts[n - 1].y);

            for pt_i in pts {
                let cur_2d = pt(pt_i.x, pt_i.y);
                let edge = segment(last_2d, cur_2d);

                if geo2d::segments_intersect(ray_seg, edge) {
                    // Fully between top and bottom → blocked.
                    if !origin_above_top && !dest_above_top && !origin_below_bot && !dest_below_bot
                    {
                        return true;
                    }

                    // Partially outside: compute ray height at intersection.
                    if let geo2d::Intersection2D::Point(ip) =
                        geo2d::segment_intersection(ray_seg, edge)
                    {
                        let ip_ground = GroundPoint::from_geo(ip);
                        let ray_z = ray_eq.z_at(ip_ground);
                        let top_z = self.compute_top_z(ip.x, ip.y);
                        let bot_z = self.compute_bottom_z(ip.x, ip.y);
                        if top_z >= ray_z && bot_z <= ray_z {
                            return true;
                        }
                    }
                }

                last_2d = cur_2d;
            }

            // Top-plane crossing for elevated obstacles: when origin and
            // destination straddle the top plane, the ray dips through
            // the "roof" inside the ground polygon.
            if origin_above_top != dest_above_top {
                let denom = origin_rel_top - dest_rel_top;
                if denom.abs() > 1e-9 {
                    let t = origin_rel_top / denom;
                    let ix = origin[0] + t * (destination[0] - origin[0]);
                    let iy = origin[1] + t * (destination[1] - origin[1]);
                    let ip = pt(ix, iy);
                    if self.box_ground.contains_point(GroundPoint::from_geo(ip))
                        && geo2d::polygon_contains_point(self.polygon.as_geo(), ip)
                    {
                        return true;
                    }
                }
            }

            // Bottom-plane crossing for elevated obstacles: when origin
            // and destination straddle the bottom plane, the ray rises
            // through the obstacle floor inside the ground polygon.
            if origin_below_bot != dest_below_bot {
                let denom = origin_rel_bot - dest_rel_bot;
                if denom.abs() > 1e-9 {
                    let t = origin_rel_bot / denom;
                    let ix = origin[0] + t * (destination[0] - origin[0]);
                    let iy = origin[1] + t * (destination[1] - origin[1]);
                    let ip = pt(ix, iy);
                    if self.box_ground.contains_point(GroundPoint::from_geo(ip))
                        && geo2d::polygon_contains_point(self.polygon.as_geo(), ip)
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Find the earliest parametric `t` along a 3D ray where this obstacle
    /// blocks it.  Returns `None` if the obstacle doesn't block the ray.
    ///
    /// `t` is in `0.0..=1.0` where 0 = origin and 1 = destination.
    /// Uses the 2D intersection point on obstacle edges to derive `t`.
    pub fn blocking_ray_3d_ratio(&self, origin: [f32; 3], destination: [f32; 3]) -> Option<f32> {
        let ray_seg = segment(pt(origin[0], origin[1]), pt(destination[0], destination[1]));

        if self.box_ground.trivially_rejects_segment(ray_seg) {
            return None;
        }

        let origin_rel_top = origin[2] - self.compute_top_z(origin[0], origin[1]);
        let dest_rel_top = destination[2] - self.compute_top_z(destination[0], destination[1]);
        let origin_above_top = origin_rel_top > 0.0;
        let dest_above_top = dest_rel_top > 0.0;

        // Both above top → no blocking.
        if origin_above_top && dest_above_top {
            return None;
        }

        // If elevated, also check below bottom.
        if !self.on_ground {
            let origin_below_bot = origin[2] - self.compute_bottom_z(origin[0], origin[1]) < 0.0;
            let dest_below_bot =
                destination[2] - self.compute_bottom_z(destination[0], destination[1]) < 0.0;
            if origin_below_bot && dest_below_bot {
                return None;
            }
        }

        let ray_eq = RayZEquation::new(origin, destination);
        let dx = destination[0] - origin[0];
        let dy = destination[1] - origin[1];
        let ray_len_sq = dx * dx + dy * dy;

        let mut min_t: Option<f32> = None;

        let pts = &self.obstacle_points;
        if pts.is_empty() {
            return None;
        }
        let n = pts.len();
        let mut last_2d = pt(pts[n - 1].x, pts[n - 1].y);

        for pt_i in pts {
            let cur_2d = pt(pt_i.x, pt_i.y);
            let edge = segment(last_2d, cur_2d);

            if geo2d::segments_intersect(ray_seg, edge)
                && let geo2d::Intersection2D::Point(ip) = geo2d::segment_intersection(ray_seg, edge)
            {
                let ip_ground = GroundPoint::from_geo(ip);
                let ray_z = ray_eq.z_at(ip_ground);
                let top_z = self.compute_top_z(ip.x, ip.y);
                let blocked = if self.on_ground {
                    if !origin_above_top && !dest_above_top {
                        true
                    } else {
                        top_z >= ray_z
                    }
                } else {
                    let bot_z = self.compute_bottom_z(ip.x, ip.y);
                    top_z >= ray_z && bot_z <= ray_z
                };

                if blocked && ray_len_sq > 1e-9 {
                    // Compute parametric t from 2D projection.
                    let ipx = ip.x - origin[0];
                    let ipy = ip.y - origin[1];
                    let t = (ipx * dx + ipy * dy) / ray_len_sq;
                    let t = t.clamp(0.0, 1.0);
                    min_t = Some(min_t.map_or(t, |prev: f32| prev.min(t)));
                }
            }

            last_2d = cur_2d;
        }

        // Top-plane crossing (both on-ground and elevated): when origin
        // is above top and destination is not, the ray dips through the
        // roof inside the ground polygon.
        if origin_above_top && !dest_above_top {
            let denom = origin_rel_top - dest_rel_top;
            if denom.abs() > 1e-9 {
                let t_plane = origin_rel_top / denom;
                let ix = origin[0] + t_plane * (destination[0] - origin[0]);
                let iy = origin[1] + t_plane * (destination[1] - origin[1]);
                let ip = pt(ix, iy);
                if self.box_ground.contains_point(GroundPoint::from_geo(ip))
                    && geo2d::polygon_contains_point(self.polygon.as_geo(), ip)
                {
                    let t = t_plane.clamp(0.0, 1.0);
                    min_t = Some(min_t.map_or(t, |prev: f32| prev.min(t)));
                }
            }
        }

        // Bottom-plane crossing for elevated obstacles: when origin is
        // below bottom and destination isn't, the ray rises through the
        // obstacle floor. The legacy implementation had a typo in the
        // denominator (mixing top and bottom relative heights); we use
        // the correct `origin_rel_bot - dest_rel_bot` linear crossing
        // formula.
        if !self.on_ground {
            let origin_rel_bot = origin[2] - self.compute_bottom_z(origin[0], origin[1]);
            let dest_rel_bot =
                destination[2] - self.compute_bottom_z(destination[0], destination[1]);
            let origin_below_bot = origin_rel_bot < 0.0;
            let dest_below_bot = dest_rel_bot < 0.0;
            if origin_below_bot && !dest_below_bot {
                let denom = origin_rel_bot - dest_rel_bot;
                if denom.abs() > 1e-9 {
                    let t_plane = origin_rel_bot / denom;
                    let ix = origin[0] + t_plane * (destination[0] - origin[0]);
                    let iy = origin[1] + t_plane * (destination[1] - origin[1]);
                    let ip = pt(ix, iy);
                    if self.box_ground.contains_point(GroundPoint::from_geo(ip))
                        && geo2d::polygon_contains_point(self.polygon.as_geo(), ip)
                    {
                        let t = t_plane.clamp(0.0, 1.0);
                        min_t = Some(min_t.map_or(t, |prev: f32| prev.min(t)));
                    }
                }
            }
        }

        min_t
    }
}

// ═══════════════════════════════════════════════════════════════════
//  3D plane helpers
// ═══════════════════════════════════════════════════════════════════

/// Compute Z height of a plane defined by 3 points at position (x, y).
///
/// Derives the plane equation `z = f(x, y)` from 3 non-degenerate points.
fn compute_plane_z(points: &[[f32; 3]; 3], x: f32, y: f32) -> f32 {
    crate::position_interface::PlaneZCoeffs::from_plane_points(points).compute_world_z(x, y)
}

/// Unit normal of a plane defined by 3 points.
///
/// Returns the zero vector if the points are degenerate (cross-product
/// magnitude is below the numerical tolerance).
fn plane_unit_normal(points: &[[f32; 3]; 3]) -> [f32; 3] {
    let [p0, p1, p2] = *points;
    let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let v2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let nx = v1[1] * v2[2] - v1[2] * v2[1];
    let ny = v1[2] * v2[0] - v1[0] * v2[2];
    let nz = v1[0] * v2[1] - v1[1] * v2[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    if len < 1e-9 {
        return [0.0, 0.0, 0.0];
    }
    [nx / len, ny / len, nz / len]
}

/// Coefficients for computing the Z height of a 3D ray at a 2D point.
///
/// Represents `Z = slope * coord + intercept` where `coord` is either
/// X or Y, chosen to be the axis with the larger extent for numerical
/// stability.
struct RayZEquation {
    slope: f32,
    intercept: f32,
    use_x: bool,
}

impl RayZEquation {
    fn new(origin: [f32; 3], destination: [f32; 3]) -> Self {
        let dx = (destination[0] - origin[0]).abs();
        let dy = (destination[1] - origin[1]).abs();
        if dx >= dy && dx > 1e-9 {
            let a = (destination[2] - origin[2]) / (destination[0] - origin[0]);
            let b = origin[2] - a * origin[0];
            Self {
                slope: a,
                intercept: b,
                use_x: true,
            }
        } else if dy > 1e-9 {
            let a = (destination[2] - origin[2]) / (destination[1] - origin[1]);
            let b = origin[2] - a * origin[1];
            Self {
                slope: a,
                intercept: b,
                use_x: false,
            }
        } else {
            // Degenerate (origin ≈ destination) — constant Z.
            Self {
                slope: 0.0,
                intercept: origin[2],
                use_x: true,
            }
        }
    }

    fn z_at(&self, p: GroundPoint) -> f32 {
        let coord = if self.use_x { p.x } else { p.y };
        self.slope * coord + self.intercept
    }
}

// ═══════════════════════════════════════════════════════════════════
//  3D reachability check
// ═══════════════════════════════════════════════════════════════════

/// Check if a 3D ray between two points is clear of SOLID|OPAQUE sight obstacles.
///
/// `type_mask` filters which obstacle types to test against (e.g.
/// `SIGHTOBSTACLE_OPAQUE` for swordfight LOS, `SIGHTOBSTACLE_SOLID |
/// SIGHTOBSTACLE_OPAQUE` for general sight checks).
///
/// Returns `true` if the path is clear, `false` if blocked.
pub fn is_reachable_3d(
    obstacles: ObstacleList<'_>,
    origin: [f32; 3],
    destination: [f32; 3],
    type_mask: u32,
) -> bool {
    // Reject rays that cross through the ground.
    if (origin[2] > 0.0 && destination[2] < 0.0) || (origin[2] < 0.0 && destination[2] > 0.0) {
        record_parity_visibility_query(origin, destination, type_mask, false);
        return false;
    }

    for (__idx, obs) in obstacles.iter_indexed() {
        if !obstacles.is_active(__idx as usize) {
            continue;
        }
        if !obs.is_of_type(type_mask) {
            continue;
        }
        if obs.is_blocking_ray_3d(origin, destination) {
            tracing::trace!(
                obstacle_index = __idx,
                obstacle_id = obs.id,
                ?origin,
                ?destination,
                type_mask,
                "3D sight ray blocked"
            );
            record_parity_visibility_query(origin, destination, type_mask, false);
            return false;
        }
    }

    record_parity_visibility_query(origin, destination, type_mask, true);
    true
}

// ═══════════════════════════════════════════════════════════════════
//  3D ray → impact-point raycast
// ═══════════════════════════════════════════════════════════════════

/// Result of a 3D raycast that hit something.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactResult3D {
    /// World-space impact point.
    pub impact: crate::coordinates::WorldPoint3D,
    /// Index of the obstacle struck, or `None` for a ground (z = 0) impact.
    pub obstacle_index: Option<u32>,
}

/// Full 3D obstacle raycast.
///
/// Casts from `origin` to `destination`, finds the nearest blocking impact
/// (wall edge, top plane, bottom plane for elevated obstacles, or ground at
/// z=0), and returns the 3D impact point plus the obstacle index.  Returns
/// `None` when the ray reaches `destination` without being blocked.
///
/// `type_filter` is the obstacle-type bitmask (`SIGHTOBSTACLE_SOLID` for
/// projectile collision, `SIGHTOBSTACLE_OPAQUE` for view, etc.).
///
/// Degenerate vertical segments (origin and destination share the same
/// `(x, y)`) short-circuit through [`is_reachable_impact_fall_3d`] or
/// [`is_reachable_impact_up_3d`] depending on direction.
pub fn is_reachable_impact_3d(
    origin: crate::coordinates::WorldPoint3D,
    destination: crate::coordinates::WorldPoint3D,
    type_filter: u32,
    obstacles: ObstacleList<'_>,
    map_bbox: Option<MapBBox>,
) -> Option<ImpactResult3D> {
    use crate::coordinates::WorldPoint3D;

    // Vertical-segment short-circuit.
    if origin.x == destination.x && origin.y == destination.y {
        return if origin.z > destination.z {
            is_reachable_impact_fall_3d(origin, destination.z, type_filter, obstacles, map_bbox)
        } else {
            is_reachable_impact_up_3d(origin, destination.z, type_filter, obstacles, map_bbox)
        };
    }

    let origin_arr = [origin.x, origin.y, origin.z];
    let dest_arr = [destination.x, destination.y, destination.z];

    // Walk active, type-matching obstacles and find the nearest impact.
    let mut best_t: f32 = f32::INFINITY;
    let mut best_obstacle: Option<u32> = None;

    for (idx, obs) in obstacles.iter_indexed().map(|(i, o)| (i as usize, o)) {
        if !obstacles.is_active(idx) {
            continue;
        }
        if !obs.is_of_type(type_filter) {
            continue;
        }
        if let Some(t) = obs.blocking_ray_3d_ratio(origin_arr, dest_arr)
            && t < best_t
        {
            best_t = t;
            best_obstacle = Some(idx as u32);
        }
    }

    // Ground-plane crossing: if the ray dips below z=0 at some t ∈ [0,1],
    // check whether that happens before any obstacle hit.
    if (origin.z >= 0.0 && destination.z < 0.0) || (origin.z < 0.0 && destination.z >= 0.0) {
        let denom = origin.z - destination.z;
        if denom.abs() > 1e-9 {
            let t_ground = origin.z / denom;
            if (0.0..=1.0).contains(&t_ground) && t_ground < best_t {
                best_t = t_ground;
                best_obstacle = None;
            }
        }
    }

    if best_t.is_finite() {
        let impact = WorldPoint3D {
            x: origin.x + best_t * (destination.x - origin.x),
            y: origin.y + best_t * (destination.y - origin.y),
            z: origin.z + best_t * (destination.z - origin.z),
        };
        Some(ImpactResult3D {
            impact,
            obstacle_index: best_obstacle,
        })
    } else {
        None
    }
}

/// Vertical-ray-downward variant of [`is_reachable_impact_3d`].
///
/// Finds the highest top-plane altitude under `origin` (among obstacles
/// whose ground polygon contains `origin`'s 2D projection) that still sits
/// between `destination_altitude` and `origin.z`.  Falls back to the ground
/// plane (`z = 0`) when `destination_altitude ≤ 0` and nothing blocks.
pub fn is_reachable_impact_fall_3d(
    origin: crate::coordinates::WorldPoint3D,
    destination_altitude: f32,
    type_filter: u32,
    obstacles: ObstacleList<'_>,
    map_bbox: Option<MapBBox>,
) -> Option<ImpactResult3D> {
    use crate::coordinates::WorldPoint3D;

    if origin.z == destination_altitude {
        return None;
    }
    if origin.z < destination_altitude {
        // Going up — no downward impact possible.
        return Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: destination_altitude,
            },
            obstacle_index: None,
        });
    }

    let p2d = GroundPoint::new(origin.x, origin.y);

    // Out-of-map guard: when origin is outside the playable rectangle,
    // force an impact at the ground plane with no obstacle.
    if let Some(bbox) = map_bbox
        && !bbox.contains_point(MapPoint::new(p2d.x, p2d.y))
    {
        return Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: 0.0,
            },
            obstacle_index: None,
        });
    }

    let mut max_top_z: f32 = 0.0;
    let mut hit_idx: Option<u32> = None;
    for (idx, obs) in obstacles.iter_indexed().map(|(i, o)| (i as usize, o)) {
        if !obstacles.is_active(idx) || !obs.is_of_type(type_filter) {
            continue;
        }
        if !obs.box_ground.contains_point(p2d) || !obs.contains_point(p2d) {
            continue;
        }
        let top = obs.compute_top_z(origin.x, origin.y);
        if top > max_top_z && top >= destination_altitude && top <= origin.z {
            max_top_z = top;
            hit_idx = Some(idx as u32);
        }
    }

    if hit_idx.is_some() {
        Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: max_top_z,
            },
            obstacle_index: hit_idx,
        })
    } else if destination_altitude <= 0.0 {
        // Ground impact at z=0.
        Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: 0.0,
            },
            obstacle_index: None,
        })
    } else {
        None
    }
}

/// Vertical-ray-upward variant of [`is_reachable_impact_3d`].
///
/// Finds the lowest bottom-plane altitude above `origin` (among obstacles
/// whose ground polygon contains `origin`'s 2D projection) that sits
/// between `origin.z` and `destination_altitude`.
pub fn is_reachable_impact_up_3d(
    origin: crate::coordinates::WorldPoint3D,
    destination_altitude: f32,
    type_filter: u32,
    obstacles: ObstacleList<'_>,
    map_bbox: Option<MapBBox>,
) -> Option<ImpactResult3D> {
    use crate::coordinates::WorldPoint3D;

    if origin.z == destination_altitude {
        return None;
    }
    if origin.z > destination_altitude {
        return Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: destination_altitude,
            },
            obstacle_index: None,
        });
    }

    let p2d = GroundPoint::new(origin.x, origin.y);

    // Out-of-map guard: when origin is outside the playable rectangle,
    // force a ground-impact with no obstacle.
    if let Some(bbox) = map_bbox
        && !bbox.contains_point(MapPoint::new(p2d.x, p2d.y))
    {
        return Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: 0.0,
            },
            obstacle_index: None,
        });
    }

    let mut min_bot_z: f32 = f32::INFINITY;
    let mut hit_idx: Option<u32> = None;
    for (idx, obs) in obstacles.iter_indexed().map(|(i, o)| (i as usize, o)) {
        if !obstacles.is_active(idx) || !obs.is_of_type(type_filter) {
            continue;
        }
        if !obs.box_ground.contains_point(p2d) || !obs.contains_point(p2d) {
            continue;
        }
        let bot = obs.compute_bottom_z(origin.x, origin.y);
        if bot < min_bot_z && bot <= destination_altitude && bot >= origin.z {
            min_bot_z = bot;
            hit_idx = Some(idx as u32);
        }
    }

    if hit_idx.is_some() {
        Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: min_bot_z,
            },
            obstacle_index: hit_idx,
        })
    } else if destination_altitude <= 0.0 {
        // Fallback when no obstacle blocks upward progress but the
        // target altitude is at/below the ground plane. This branch is
        // not expected under normal play (an upward ray ending at/below
        // ground implies origin is also at/below ground), so we log a
        // warning rather than panic to degrade gracefully.
        tracing::warn!(
            "is_reachable_impact_up_3d: origin.z={} <= dest_alt={} <= 0 with no blocker; returning ground impact",
            origin.z,
            destination_altitude,
        );
        Some(ImpactResult3D {
            impact: WorldPoint3D {
                x: origin.x,
                y: origin.y,
                z: 0.0,
            },
            obstacle_index: None,
        })
    } else {
        None
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple square obstacle at (0,0)..(10,10) with z 0..5.
    /// `top_plane_points` and `bottom_plane_points` are set separately
    /// (the level-loader writes them from the SGHT/WOAW chunk; the Rust
    /// `rebuild_geometry` doesn't derive them from `obstacle_points`).
    fn make_square_obstacle() -> SightObstacle {
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 10.0,
                z_top: 5.0,
                z_bottom: 0.0,
            },
        ];
        obs.top_plane_points = [[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [0.0, 10.0, 5.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        obs.rebuild_geometry();
        obs
    }

    #[test]
    fn default_type_flags() {
        let obs = SightObstacle::new_default(1);
        assert!(obs.is_solid());
        assert!(obs.is_opaque());
        assert!(!obs.is_projection_area());
        assert!(!obs.is_mouse());
        assert!(!obs.is_shield());
        assert!(!obs.is_showing_shadow_polygon());
    }

    #[test]
    fn set_flags() {
        let mut obs = SightObstacle::new(0, 0);
        assert!(!obs.is_solid());
        obs.set_flag(SIGHTOBSTACLE_SOLID, true);
        assert!(obs.is_solid());
        obs.set_flag(SIGHTOBSTACLE_SOLID, false);
        assert!(!obs.is_solid());
    }

    #[test]
    fn obstacle_list_active() {
        // Active flags now live in `ObstacleList::static_active`,
        // parallel to the static slice.
        let obs = SightObstacle::new_default(0);
        let static_obs = vec![obs];
        let list = ObstacleList {
            static_obstacles: &static_obs,
            dynamic_obstacles: &[],
            static_active: &[true],
        };
        assert!(list.is_active(0));
        let list = ObstacleList {
            static_obstacles: &static_obs,
            dynamic_obstacles: &[],
            static_active: &[false],
        };
        assert!(!list.is_active(0));
    }

    #[test]
    fn contains_point_inside() {
        let obs = make_square_obstacle();
        assert!(obs.contains_point(GroundPoint::new(5.0, 5.0)));
        assert!(obs.contains_point(GroundPoint::new(0.0, 0.0))); // boundary
    }

    #[test]
    fn contains_point_outside() {
        let obs = make_square_obstacle();
        assert!(!obs.contains_point(GroundPoint::new(-1.0, 5.0)));
        assert!(!obs.contains_point(GroundPoint::new(15.0, 5.0)));
    }

    #[test]
    fn blocking_sight_through_obstacle() {
        let obs = make_square_obstacle();
        // Line from left of obstacle to right of obstacle — crosses polygon.
        assert!(obs.is_blocking_sight(GroundPoint::new(-5.0, 5.0), GroundPoint::new(15.0, 5.0)));
    }

    #[test]
    fn not_blocking_sight_around_obstacle() {
        let obs = make_square_obstacle();
        // Line that passes above the obstacle (in ground Y).
        assert!(!obs.is_blocking_sight(GroundPoint::new(-5.0, 15.0), GroundPoint::new(15.0, 15.0)));
    }

    #[test]
    fn rebuild_geometry_sets_bboxes() {
        let obs = make_square_obstacle();
        assert!(obs.box_ground.is_somewhere());
        assert!((obs.box_3d_min[2] - 0.0).abs() < 1e-6);
        assert!((obs.box_3d_max[2] - 5.0).abs() < 1e-6);
        assert!(obs.on_ground);
    }

    #[test]
    fn elevated_obstacle_not_on_ground() {
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
        ];
        obs.rebuild_geometry();
        assert!(!obs.on_ground);
    }

    #[test]
    fn translate_2d_moves_polygon() {
        let mut obs = make_square_obstacle();
        obs.translate_2d(100.0, 100.0);
        assert!(obs.contains_point(GroundPoint::new(105.0, 105.0)));
        assert!(!obs.contains_point(GroundPoint::new(5.0, 5.0)));
    }

    #[test]
    fn serde_roundtrip() {
        let obs = make_square_obstacle();
        let json = serde_json::to_string(&obs).expect("serialize");
        let deser: SightObstacle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deser.id, obs.id);
        assert_eq!(deser.obstacle_type, obs.obstacle_type);
        assert_eq!(deser.obstacle_points.len(), obs.obstacle_points.len());
    }

    // ── 3D ray blocking tests ──

    /// Make a flat-topped obstacle at z_top=5 with proper plane points.
    fn make_flat_obstacle() -> SightObstacle {
        let mut obs = make_square_obstacle();
        // Flat top plane at z=5 (3 points on z=5).
        obs.top_plane_points = [[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [0.0, 10.0, 5.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        obs
    }

    #[test]
    fn ray_3d_through_wall_blocked() {
        let obs = make_flat_obstacle();
        // Ray at z=2.5 going through the obstacle — should be blocked.
        assert!(obs.is_blocking_ray_3d([-5.0, 5.0, 2.5], [15.0, 5.0, 2.5]));
    }

    #[test]
    fn ray_3d_over_wall_clear() {
        let obs = make_flat_obstacle();
        // Ray at z=10 going over the z=5 obstacle — should be clear.
        assert!(!obs.is_blocking_ray_3d([-5.0, 5.0, 10.0], [15.0, 5.0, 10.0]));
    }

    #[test]
    fn ray_starting_on_sloped_plane_uses_original_float_rounding() {
        // Linux Savegame_034: the bonus-to-PC ray starts on the large
        // ground plane. The algebraically equivalent unnormalized plane
        // equation rounded its Z one ULP above the origin and falsely
        // classified this otherwise-clear ray as passing through obstacle 90.
        let mut obs = SightObstacle::new_default(90);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 1300.0,
                y: 1900.0,
                z_top: 100.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 2050.0,
                y: 1900.0,
                z_top: 100.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 2050.0,
                y: 2420.0,
                z_top: 100.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 1300.0,
                y: 2420.0,
                z_top: 100.0,
                z_bottom: 0.0,
            },
        ];
        obs.top_plane_points = [
            [1796.8242, 2317.0935, 0.19600001],
            [1301.3945, 2408.703, 90.00101],
            [1995.4719, 1911.5131, 0.001],
        ];
        obs.rebuild_geometry();

        let origin = [1921.0, 1941.8881, 11.888083];
        let destination = [2009.9403, 1955.6993, 45.0];
        assert!(origin[2] > obs.compute_top_z(origin[0], origin[1]));
        assert!(!obs.is_blocking_ray_3d(origin, destination));
    }

    #[test]
    fn ray_3d_arcing_over_wall_clear() {
        let obs = make_flat_obstacle();
        // Ray starting low, ending high — arc goes over obstacle.
        // At x=-5 z=2, at x=15 z=8. At x=0 (wall entry): z = 2 + 6*(5/20) = 3.5
        // At x=10 (wall exit): z = 2 + 6*(15/20) = 6.5. But we need
        // to check properly — the ray rises from z=2 to z=8 linearly.
        // At the two edges (x=0: z=3.5) and (x=10: z=6.5).
        // Top plane z=5. At x=0, ray z=3.5 < 5 → blocked at first edge!
        assert!(obs.is_blocking_ray_3d([-5.0, 5.0, 2.0], [15.0, 5.0, 8.0]));
    }

    #[test]
    fn ray_3d_high_arc_over_wall_clear() {
        let obs = make_flat_obstacle();
        // Both endpoints above z=5: start z=6, end z=6 — passes over.
        assert!(!obs.is_blocking_ray_3d([-5.0, 5.0, 6.0], [15.0, 5.0, 6.0]));
    }

    #[test]
    fn ray_3d_descending_into_wall_blocked() {
        let obs = make_flat_obstacle();
        // Start above (z=7), end below top (z=2) — crosses top plane inside polygon.
        assert!(obs.is_blocking_ray_3d([-5.0, 5.0, 7.0], [15.0, 5.0, 2.0]));
    }

    #[test]
    fn ray_3d_around_wall_clear() {
        let obs = make_flat_obstacle();
        // Ray that goes around (above in Y) the obstacle.
        assert!(!obs.is_blocking_ray_3d([-5.0, 15.0, 2.5], [15.0, 15.0, 2.5]));
    }

    #[test]
    fn is_reachable_3d_inactive_skipped() {
        let obs = make_flat_obstacle();
        let obstacles = [obs];
        // With the active flag explicitly false in the obstacle list,
        // is_reachable_3d should treat the ray as clear.
        let list = ObstacleList {
            static_obstacles: &obstacles,
            dynamic_obstacles: &[],
            static_active: &[false],
        };
        assert!(is_reachable_3d(
            list,
            [-5.0, 5.0, 2.5],
            [15.0, 5.0, 2.5],
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE
        ));
    }

    #[test]
    fn is_reachable_3d_multiple_obstacles() {
        let obs1 = make_flat_obstacle();
        let obstacles = [obs1];
        // Blocked ray.
        assert!(!is_reachable_3d(
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
            [-5.0, 5.0, 2.5],
            [15.0, 5.0, 2.5],
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE
        ));
        // Clear ray (above).
        assert!(is_reachable_3d(
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles),
            [-5.0, 5.0, 10.0],
            [15.0, 5.0, 10.0],
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE
        ));
    }

    #[test]
    fn combined_type_filter_requires_every_requested_bit() {
        // H05_Lin_EC obstacle 245 has this exact relevant type shape: it is
        // solid shadow geometry but is not opaque. Original
        // RHSightObstacle::IsOfType(SOLID | OPAQUE) excludes it.
        let mut solid_only = make_flat_obstacle();
        solid_only.obstacle_type =
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_MOUSE | SIGHTOBSTACLE_SHOW_SHADOW_POLYGON;
        assert!(solid_only.is_of_type(SIGHTOBSTACLE_SOLID));
        assert!(!solid_only.is_of_type(SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE));

        let obstacles = [solid_only];
        let list = ObstacleList::from_slice_all_active(&obstacles);
        let origin = [-5.0, 5.0, 2.5];
        let destination = [15.0, 5.0, 2.5];

        assert!(is_reachable_3d(
            list,
            origin,
            destination,
            SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_OPAQUE,
        ));
        assert!(!is_reachable_3d(
            list,
            origin,
            destination,
            SIGHTOBSTACLE_SOLID,
        ));
    }

    #[test]
    fn compute_plane_z_flat() {
        // Flat plane at z=5.
        let pts = [[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [0.0, 10.0, 5.0]];
        assert!((compute_plane_z(&pts, 5.0, 5.0) - 5.0).abs() < 1e-4);
        assert!((compute_plane_z(&pts, 0.0, 0.0) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn compute_plane_z_sloped() {
        // Sloped plane: z = 2 + 0.3*x.
        let pts = [[0.0, 0.0, 2.0], [10.0, 0.0, 5.0], [0.0, 10.0, 2.0]];
        assert!((compute_plane_z(&pts, 10.0, 0.0) - 5.0).abs() < 1e-4);
        assert!((compute_plane_z(&pts, 5.0, 0.0) - 3.5).abs() < 1e-4);
    }

    #[test]
    fn elevated_obstacle_ray_between_planes_blocked() {
        // Elevated obstacle: bottom=3, top=10.
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
        ];
        obs.top_plane_points = [[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [0.0, 10.0, 10.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 3.0], [10.0, 0.0, 3.0], [0.0, 10.0, 3.0]];
        obs.rebuild_geometry();

        // Ray at z=6 (between bottom=3 and top=10) — blocked.
        assert!(obs.is_blocking_ray_3d([-5.0, 5.0, 6.0], [15.0, 5.0, 6.0]));
        // Ray at z=1 (below bottom=3) — clear.
        assert!(!obs.is_blocking_ray_3d([-5.0, 5.0, 1.0], [15.0, 5.0, 1.0]));
        // Ray at z=12 (above top=10) — clear.
        assert!(!obs.is_blocking_ray_3d([-5.0, 5.0, 12.0], [15.0, 5.0, 12.0]));
    }

    // ── is_reachable_impact_3d ────────────────────────────────

    use crate::coordinates::WorldPoint3D;

    #[test]
    fn impact_3d_clear_path() {
        // No obstacles in the way — returns None (clear).
        let obs = make_square_obstacle();
        let origin = WorldPoint3D {
            x: 20.0,
            y: 5.0,
            z: 1.0,
        };
        let dest = WorldPoint3D {
            x: 30.0,
            y: 5.0,
            z: 1.0,
        };
        assert!(
            is_reachable_impact_3d(
                origin,
                dest,
                SIGHTOBSTACLE_SOLID,
                ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn impact_3d_wall_impact_point() {
        // Ray passes through a 10x10 wall (z 0..5) at y=5.  Entering from
        // the left at (-5, 5, 1) heading to (15, 5, 1).  Expected impact
        // at the first wall (x=0), z stays at 1.
        let obs = make_square_obstacle();
        let origin = WorldPoint3D {
            x: -5.0,
            y: 5.0,
            z: 1.0,
        };
        let dest = WorldPoint3D {
            x: 15.0,
            y: 5.0,
            z: 1.0,
        };
        let result = is_reachable_impact_3d(
            origin,
            dest,
            SIGHTOBSTACLE_SOLID,
            ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
            None,
        )
        .unwrap();
        assert_eq!(result.obstacle_index, Some(0));
        assert!((result.impact.x - 0.0).abs() < 1e-3);
        assert!((result.impact.y - 5.0).abs() < 1e-3);
        assert!((result.impact.z - 1.0).abs() < 1e-3);
    }

    #[test]
    fn impact_3d_ground_crossing() {
        // Ray from z=5 down to z=-5 at x=20 (outside any obstacle) —
        // impacts the ground plane (z=0) at the midpoint.
        let obs = make_square_obstacle();
        let origin = WorldPoint3D {
            x: 20.0,
            y: 5.0,
            z: 5.0,
        };
        let dest = WorldPoint3D {
            x: 20.0,
            y: 5.0,
            z: -5.0,
        };
        let result = is_reachable_impact_3d(
            origin,
            dest,
            SIGHTOBSTACLE_SOLID,
            ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
            None,
        )
        .unwrap();
        // Vertical-segment short-circuit routes this through fall_3d —
        // the 2D projection is outside the obstacle, so it reports ground.
        assert_eq!(result.obstacle_index, None);
        assert!((result.impact.z - 0.0).abs() < 1e-3);
    }

    #[test]
    fn impact_3d_fall_onto_roof() {
        // Vertical fall onto the top of the square obstacle.
        let obs = make_square_obstacle();
        let origin = WorldPoint3D {
            x: 5.0,
            y: 5.0,
            z: 20.0,
        };
        let dest = WorldPoint3D {
            x: 5.0,
            y: 5.0,
            z: 0.0,
        };
        let result = is_reachable_impact_3d(
            origin,
            dest,
            SIGHTOBSTACLE_SOLID,
            ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
            None,
        )
        .unwrap();
        assert_eq!(result.obstacle_index, Some(0));
        // Top of obstacle is z=5.
        assert!((result.impact.z - 5.0).abs() < 1e-3);
    }

    #[test]
    fn impact_3d_rising_into_obstacle_floor() {
        // Elevated obstacle floor at z=3, vertical rise from z=0 to z=10
        // directly below — should impact the bottom plane.
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 3.0,
            },
        ];
        obs.top_plane_points = [[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [0.0, 10.0, 10.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 3.0], [10.0, 0.0, 3.0], [0.0, 10.0, 3.0]];
        obs.rebuild_geometry();

        let origin = WorldPoint3D {
            x: 5.0,
            y: 5.0,
            z: 0.0,
        };
        let dest = WorldPoint3D {
            x: 5.0,
            y: 5.0,
            z: 10.0,
        };
        let result = is_reachable_impact_3d(
            origin,
            dest,
            SIGHTOBSTACLE_SOLID,
            ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
            None,
        )
        .unwrap();
        assert_eq!(result.obstacle_index, Some(0));
        assert!((result.impact.z - 3.0).abs() < 1e-3);
    }

    #[test]
    fn impact_3d_filter_excludes_non_matching_types() {
        // Obstacle is OPAQUE only — querying with SIGHTOBSTACLE_SOLID
        // filter should miss it.
        let mut obs = make_square_obstacle();
        obs.obstacle_type = SIGHTOBSTACLE_OPAQUE;
        let origin = WorldPoint3D {
            x: -5.0,
            y: 5.0,
            z: 1.0,
        };
        let dest = WorldPoint3D {
            x: 15.0,
            y: 5.0,
            z: 1.0,
        };
        assert!(
            is_reachable_impact_3d(
                origin,
                dest,
                SIGHTOBSTACLE_SOLID,
                ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
                None,
            )
            .is_none()
        );
        // But with the OPAQUE filter, it blocks.
        assert!(
            is_reachable_impact_3d(
                origin,
                dest,
                SIGHTOBSTACLE_OPAQUE,
                ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
                None,
            )
            .is_some()
        );
    }

    #[test]
    fn impact_3d_fall_clear_when_no_obstacle_below() {
        // Origin above (no obstacle under 2D projection), destination high
        // — `is_reachable_impact_fall_3d` returns None (clear path).
        let obs = make_square_obstacle();
        let origin = WorldPoint3D {
            x: 20.0,
            y: 5.0,
            z: 10.0,
        };
        let result = is_reachable_impact_fall_3d(
            origin,
            5.0,
            SIGHTOBSTACLE_SOLID,
            ObstacleList::from_slice_all_active(std::slice::from_ref(&obs)),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn parity_capture_records_only_ordered_opaque_queries() {
        let obstacles = ObstacleList::from_slice_all_active(&[]);
        begin_parity_visibility_capture();
        assert!(is_reachable_3d(
            obstacles,
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            SIGHTOBSTACLE_OPAQUE,
        ));
        assert!(is_reachable_3d(
            obstacles,
            [7.0, 8.0, 9.0],
            [10.0, 11.0, 12.0],
            SIGHTOBSTACLE_SOLID,
        ));
        assert!(!is_reachable_3d(
            obstacles,
            [13.0, 14.0, 1.0],
            [15.0, 16.0, -1.0],
            SIGHTOBSTACLE_OPAQUE,
        ));

        assert_eq!(
            take_parity_visibility_capture(),
            [
                ParityVisibilityQuery {
                    origin: [1.0, 2.0, 3.0],
                    destination: [4.0, 5.0, 6.0],
                    result: true,
                },
                ParityVisibilityQuery {
                    origin: [13.0, 14.0, 1.0],
                    destination: [15.0, 16.0, -1.0],
                    result: false,
                },
            ]
        );
    }
}
