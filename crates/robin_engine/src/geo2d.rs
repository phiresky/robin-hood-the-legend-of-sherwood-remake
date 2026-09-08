//! 2D geometry adapter layer.
//!
//! Wraps the `geo` crate's types with the primitives the engine needs:
//! `GeoPoint2D`, `Vec2D`, `BBox2D` (axis-aligned bounding box with a
//! "hyperspace"/unset state represented by `None`), `Segment2D`,
//! `Line2D` (infinite line), `HalfLine2D` (ray), and `Polygon2D`. All
//! coordinates are `f32`, with a precision tolerance of [`PRECISION`]
//! (`1e-9`).

use serde::{Deserialize, Serialize};

use geo::{
    BoundingRect, ClosestPoint, Contains, Coord, Intersects, Line, LineString, Polygon, Rect,
};

/// Float precision tolerance for geometric comparisons.
pub const PRECISION: f32 = 1e-9;

// Re-export geo types that callers will use directly.
pub use geo::Line as Segment2D;
pub use geo::LineString as PolyLine2D;
pub use geo::Polygon as Polygon2D;

// ─── Point / Vector ──────────────────────────────────────────────

/// A 2D point.
pub type GeoPoint2D = Coord<f32>;

/// A 2D vector. Same underlying type as [`GeoPoint2D`] since `geo::Coord`
/// supports arithmetic.
pub type Vec2D = Coord<f32>;

/// Construct a point.
#[inline]
pub fn pt(x: f32, y: f32) -> GeoPoint2D {
    Coord { x, y }
}

/// Dot product of two vectors.
#[inline]
pub fn dot(a: Vec2D, b: Vec2D) -> f32 {
    a.x * b.x + a.y * b.y
}

/// 2D cross product (determinant): a × b.
#[inline]
pub fn cross(a: Vec2D, b: Vec2D) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Euclidean length of a vector.
#[inline]
pub fn length(v: Vec2D) -> f32 {
    (v.x * v.x + v.y * v.y).sqrt()
}

/// Normalize a vector. Returns zero vector if length is below precision.
#[inline]
pub fn normalize(v: Vec2D) -> Vec2D {
    let len = length(v);
    if len < PRECISION {
        pt(0.0, 0.0)
    } else {
        pt(v.x / len, v.y / len)
    }
}

/// Rotate a point around a center by `angle` radians.
pub fn rotate_around(p: GeoPoint2D, center: GeoPoint2D, angle: f32) -> GeoPoint2D {
    let dx = p.x - center.x;
    let dy = p.y - center.y;
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    pt(
        center.x + dx * cos_a - dy * sin_a,
        center.y + dx * sin_a + dy * cos_a,
    )
}

/// Test if two points are within `epsilon` of each other.
///
/// Uses `|dx| < ε && |dy| < ε` — an open axis-aligned square of side `2ε`
/// (Chebyshev / L∞ ball with strict comparator), not an L2 disc.
#[inline]
pub fn points_near(a: GeoPoint2D, b: GeoPoint2D, epsilon: f32) -> bool {
    (a.x - b.x).abs() < epsilon && (a.y - b.y).abs() < epsilon
}

// ─── BoundingBox2D ───────────────────────────────────────────────

/// Axis-aligned bounding box.
///
/// `None` represents the "hyperspace" state (bounds not set).
/// `Some(rect)` holds the actual box.
/// With geo's `serde` feature enabled, `Coord<f32>` and `Rect<f32>`
/// implement Serialize/Deserialize natively.

#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct BBox2D(pub Option<Rect<f32>>);

crate::bitcode_adapters::impl_native_bitcode_rect!(BBox2D);

impl BBox2D {
    /// Empty box ("hyperspace" — bounds not set).
    pub fn new() -> Self {
        BBox2D(None)
    }

    /// Box from min/max corners.
    pub fn from_corners(min: GeoPoint2D, max: GeoPoint2D) -> Self {
        BBox2D(Some(Rect::new(min, max)))
    }

    /// Box from explicit coordinates.
    pub fn from_coords(x_min: f32, y_min: f32, x_max: f32, y_max: f32) -> Self {
        BBox2D(Some(Rect::new(pt(x_min, y_min), pt(x_max, y_max))))
    }

    /// Box around a single point.
    pub fn from_point(p: GeoPoint2D) -> Self {
        BBox2D(Some(Rect::new(p, p)))
    }

    /// Box from a point + width/height.
    pub fn from_point_size(origin: GeoPoint2D, width: f32, height: f32) -> Self {
        let x0 = origin.x.min(origin.x + width);
        let y0 = origin.y.min(origin.y + height);
        let x1 = origin.x.max(origin.x + width);
        let y1 = origin.y.max(origin.y + height);
        BBox2D(Some(Rect::new(pt(x0, y0), pt(x1, y1))))
    }

    /// Whether the box has defined bounds.
    #[inline]
    pub fn is_somewhere(&self) -> bool {
        self.0.is_some()
    }

    /// Reset to hyperspace.
    pub fn reset(&mut self) {
        self.0 = None;
    }

    // ── Getters ──

    #[inline]
    pub fn x_min(&self) -> f32 {
        self.0.unwrap().min().x
    }
    #[inline]
    pub fn y_min(&self) -> f32 {
        self.0.unwrap().min().y
    }
    #[inline]
    pub fn x_max(&self) -> f32 {
        self.0.unwrap().max().x
    }
    #[inline]
    pub fn y_max(&self) -> f32 {
        self.0.unwrap().max().y
    }
    #[inline]
    pub fn width(&self) -> f32 {
        let r = self.0.unwrap();
        r.max().x - r.min().x
    }
    #[inline]
    pub fn center(&self) -> GeoPoint2D {
        let r = self.0.unwrap();
        pt((r.min().x + r.max().x) * 0.5, (r.min().y + r.max().y) * 0.5)
    }
    #[inline]
    pub fn top_left(&self) -> GeoPoint2D {
        self.0.unwrap().min()
    }
    #[inline]
    pub fn bottom_right(&self) -> GeoPoint2D {
        self.0.unwrap().max()
    }

    // ── Expand ──

    /// Expand to include a point. If the box is in hyperspace, it drops
    /// down to that point.
    pub fn expand_point(&mut self, p: GeoPoint2D) {
        match &mut self.0 {
            None => {
                self.0 = Some(Rect::new(p, p));
            }
            Some(r) => {
                let min = pt(r.min().x.min(p.x), r.min().y.min(p.y));
                let max = pt(r.max().x.max(p.x), r.max().y.max(p.y));
                *r = Rect::new(min, max);
            }
        }
    }

    /// Expand to include another bounding box.
    ///
    /// If `other` is in hyperspace this is a no-op — we early-return on
    /// the hyperspace arm rather than treating an unset box as `(0,0)`
    /// corners, which would silently drag `self` to include the origin.
    pub fn expand_bbox(&mut self, other: &BBox2D) {
        if let Some(r) = other.0 {
            self.expand_point(r.min());
            self.expand_point(r.max());
        }
    }

    // ── Tests ──

    /// Test if a point is inside the box (boundary included).
    pub fn contains_point(&self, p: GeoPoint2D) -> bool {
        match self.0 {
            None => false,
            Some(r) => p.x >= r.min().x && p.x <= r.max().x && p.y >= r.min().y && p.y <= r.max().y,
        }
    }

    /// Half-open hit-test: inclusive on the top-left edges, **strict**
    /// on the bottom-right edges (`x < x_max`, `y < y_max`). Use this
    /// for widget mouse hit-tests so adjacent widgets never both claim
    /// the shared right/bottom pixel column. [`Self::contains_point`] is
    /// the closed variant.
    pub fn is_boxed_point(&self, p: GeoPoint2D) -> bool {
        match self.0 {
            None => false,
            Some(r) => p.x >= r.min().x && p.x < r.max().x && p.y >= r.min().y && p.y < r.max().y,
        }
    }

    /// Test if a point lies on one of the four edges of the box.
    ///
    /// True iff `x == x_min` or `x == x_max` (with the other axis in
    /// `[y_min, y_max]`), or `y == y_min` or `y == y_max` (with the
    /// other axis in `[x_min, x_max]`). Returns `false` on a hyperspace
    /// box.
    pub fn is_on_boundary(&self, p: GeoPoint2D) -> bool {
        let r = match self.0 {
            None => return false,
            Some(r) => r,
        };
        let (x_min, x_max) = (r.min().x, r.max().x);
        let (y_min, y_max) = (r.min().y, r.max().y);
        if p.x == x_min || p.x == x_max {
            return y_min <= p.y && p.y <= y_max;
        }
        if p.y == y_min || p.y == y_max {
            return x_min <= p.x && p.x <= x_max;
        }
        false
    }

    /// Test if another box is fully inside this box.
    pub fn contains_bbox(&self, other: &BBox2D) -> bool {
        match (self.0, other.0) {
            (Some(a), Some(b)) => a.contains(&b),
            _ => false,
        }
    }

    /// Trivial rejection test for a segment (both endpoints on the
    /// same rejecting side).
    pub fn trivially_rejects_segment(&self, seg: Line<f32>) -> bool {
        match self.0 {
            None => true,
            Some(r) => {
                let (a, b) = (seg.start, seg.end);
                (a.x < r.min().x && b.x < r.min().x)
                    || (a.x > r.max().x && b.x > r.max().x)
                    || (a.y < r.min().y && b.y < r.min().y)
                    || (a.y > r.max().y && b.y > r.max().y)
            }
        }
    }

    /// Test if a segment intersects the box.
    pub fn intersects_segment(&self, seg: Line<f32>) -> bool {
        match self.0 {
            None => false,
            Some(r) => r.intersects(&seg),
        }
    }

    /// Test if another box intersects this box.
    pub fn intersects_bbox(&self, other: &BBox2D) -> bool {
        match (self.0, other.0) {
            (Some(a), Some(b)) => a.intersects(&b),
            _ => false,
        }
    }

    // ── Clip ──

    /// Clip this box against another, returning the intersection box
    /// (or None if no overlap).
    pub fn clip_bbox(&self, other: &BBox2D) -> BBox2D {
        match (self.0, other.0) {
            (Some(a), Some(b)) => {
                let x_min = a.min().x.max(b.min().x);
                let y_min = a.min().y.max(b.min().y);
                let x_max = a.max().x.min(b.max().x);
                let y_max = a.max().y.min(b.max().y);
                if x_min <= x_max && y_min <= y_max {
                    BBox2D::from_coords(x_min, y_min, x_max, y_max)
                } else {
                    BBox2D::new()
                }
            }
            _ => BBox2D::new(),
        }
    }

    // ── Translation ──

    /// Translate the box by a vector.
    pub fn translate(&mut self, v: Vec2D) {
        if let Some(r) = &mut self.0 {
            let min = pt(r.min().x + v.x, r.min().y + v.y);
            let max = pt(r.max().x + v.x, r.max().y + v.y);
            *r = Rect::new(min, max);
        }
    }

    /// Return a translated copy.
    pub fn translated(&self, v: Vec2D) -> BBox2D {
        let mut b = *self;
        b.translate(v);
        b
    }
}

impl Default for BBox2D {
    fn default() -> Self {
        Self::new()
    }
}

/// Construct a BBox from a `geo::Polygon`.
impl From<&Polygon<f32>> for BBox2D {
    fn from(poly: &Polygon<f32>) -> Self {
        match poly.bounding_rect() {
            Some(r) => BBox2D(Some(r)),
            None => BBox2D::new(),
        }
    }
}

/// Construct a BBox from a `geo::LineString`.
impl From<&LineString<f32>> for BBox2D {
    fn from(ls: &LineString<f32>) -> Self {
        match ls.bounding_rect() {
            Some(r) => BBox2D(Some(r)),
            None => BBox2D::new(),
        }
    }
}

// ─── Segment helpers ─────────────────────────────────────────────

/// Construct a segment from two points.
#[inline]
pub fn segment(a: GeoPoint2D, b: GeoPoint2D) -> Line<f32> {
    Line::new(a, b)
}

/// Euclidean distance between two points.
#[inline]
pub fn distance(a: GeoPoint2D, b: GeoPoint2D) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

// ─── Legacy authored-data I/O ────────────────────────────────────

impl BBox2D {
    /// Read an original-game serialized bounding box.
    /// Format: 4 x f32 (top_left.x, top_left.y, bottom_right.x, bottom_right.y) + 1 x bool.
    pub fn read_legacy(
        reader: &mut crate::legacy_io::LegacyReader<'_>,
        field: impl std::fmt::Display,
    ) -> crate::legacy_io::LegacyResult<Self> {
        reader.scope(field.to_string(), |reader| {
            let tl_x = reader.read_f32("top_left.x")?;
            let tl_y = reader.read_f32("top_left.y")?;
            let br_x = reader.read_f32("bottom_right.x")?;
            let br_y = reader.read_f32("bottom_right.y")?;
            let bounds_set = reader.read_bool("bounds_set")?;

            if bounds_set {
                Ok(BBox2D::from_coords(tl_x, tl_y, br_x, br_y))
            } else {
                Ok(BBox2D::new())
            }
        })
    }

    /// Write the exact original-game bounding-box layout.
    pub fn write_legacy<W: std::io::Write>(
        &self,
        writer: &mut crate::legacy_io::LegacyWriter<W>,
        field: impl std::fmt::Display,
    ) -> crate::legacy_io::LegacyResult<()> {
        writer.scope(field.to_string(), |writer| {
            writer.write_f32("top_left.x", self.0.map_or(0.0, |r| r.min().x))?;
            writer.write_f32("top_left.y", self.0.map_or(0.0, |r| r.min().y))?;
            writer.write_f32("bottom_right.x", self.0.map_or(0.0, |r| r.max().x))?;
            writer.write_f32("bottom_right.y", self.0.map_or(0.0, |r| r.max().y))?;
            writer.write_bool("bounds_set", self.is_somewhere())
        })
    }
}

/// Read a GeoPoint2D. Format: 2 x f32 (x, y).
pub fn read_legacy_geo_point(
    reader: &mut crate::legacy_io::LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> crate::legacy_io::LegacyResult<GeoPoint2D> {
    reader.scope(field.to_string(), |reader| {
        Ok(pt(reader.read_f32("x")?, reader.read_f32("y")?))
    })
}

/// Write a GeoPoint2D in the exact original-game layout.
pub fn write_legacy_geo_point<W: std::io::Write>(
    writer: &mut crate::legacy_io::LegacyWriter<W>,
    field: impl std::fmt::Display,
    point: GeoPoint2D,
) -> crate::legacy_io::LegacyResult<()> {
    writer.scope(field.to_string(), |writer| {
        writer.write_f32("x", point.x)?;
        writer.write_f32("y", point.y)
    })
}

// ─── Segment operations ──────────────────────────────────────────

/// Test if two segments intersect.
///
/// Uses f32 cross-product determinants throughout — avoids the `geo`
/// crate's `robust::orient2d::<f64>` path, which dominates AI sight-check
/// profiles.
#[inline]
pub fn segments_intersect(a: Line<f32>, b: Line<f32>) -> bool {
    // AABB rejection.
    let (ax_lo, ax_hi) = if a.start.x < a.end.x {
        (a.start.x, a.end.x)
    } else {
        (a.end.x, a.start.x)
    };
    let (ay_lo, ay_hi) = if a.start.y < a.end.y {
        (a.start.y, a.end.y)
    } else {
        (a.end.y, a.start.y)
    };
    let (bx_lo, bx_hi) = if b.start.x < b.end.x {
        (b.start.x, b.end.x)
    } else {
        (b.end.x, b.start.x)
    };
    let (by_lo, by_hi) = if b.start.y < b.end.y {
        (b.start.y, b.end.y)
    } else {
        (b.end.y, b.start.y)
    };
    if ax_hi < bx_lo || bx_hi < ax_lo || ay_hi < by_lo || by_hi < ay_lo {
        return false;
    }

    // vector1 = a.start - a.end, vector2 = b.end - b.start.
    let v1x = a.start.x - a.end.x;
    let v1y = a.start.y - a.end.y;
    let v2x = b.end.x - b.start.x;
    let v2y = b.end.y - b.start.y;

    // vectorA1A2 = b.start - a.start.
    let a1a2_x = b.start.x - a.start.x;
    let a1a2_y = b.start.y - a.start.y;

    // det1 = cross(vector1, vectorA1A2);  det2 = cross(vector1, b.end - a.start);
    let det1 = v1x * a1a2_y - v1y * a1a2_x;
    let det2 = v1x * (b.end.y - a.start.y) - v1y * (b.end.x - a.start.x);
    if !((det1 >= 0.0 && det2 <= 0.0) || (det1 <= 0.0 && det2 >= 0.0)) {
        return false;
    }

    // det1 = cross(vector2, vectorA1A2);  det2 = cross(vector2, a.end - b.start);
    let det1 = v2x * a1a2_y - v2y * a1a2_x;
    let det2 = v2x * (a.end.y - b.start.y) - v2y * (a.end.x - b.start.x);
    (det1 <= 0.0 && det2 <= 0.0) || (det1 >= 0.0 && det2 >= 0.0)
}

/// Guarded crossing test — [`segments_intersect`] plus a two-det guard.
///
/// Reports `false` when `seg.start` lies on the infinite line extension
/// of `line`, so a single crossing cannot fire twice on consecutive
/// ticks when the actor's old position ends up exactly on the line.
#[inline]
pub fn is_crossed(line: Line<f32>, seg: Line<f32>) -> bool {
    if !segments_intersect(line, seg) {
        return false;
    }
    let vtest_x = seg.start.x - line.start.x;
    let vtest_y = seg.start.y - line.start.y;
    // line vector = line.end - line.start
    let lvx = line.end.x - line.start.x;
    let lvy = line.end.y - line.start.y;
    // seg vector = seg.end - seg.start
    let svx = seg.end.x - seg.start.x;
    let svy = seg.end.y - seg.start.y;
    let line_det = lvx * vtest_y - lvy * vtest_x;
    let seg_det = svx * vtest_y - svy * vtest_x;
    line_det != 0.0 && seg_det != 0.0
}

/// Test if a segment intersects a polygon's boundary (any exterior edge).
///
/// Walks consecutive edges using the same f32 cross-product test as
/// [`segments_intersect`]. This is the hot path for AI sight-obstacle
/// blocking checks.
///
/// Note: returns false when the segment is entirely *inside* the polygon
/// without touching any edge. Sight-obstacle queries never put viewer
/// and target both inside the same wall, so this matches usage.
pub fn segment_intersects_polygon(seg: Line<f32>, poly: &Polygon<f32>) -> bool {
    segment_intersects_linestring(seg, poly.exterior())
}

#[inline]
fn segment_intersects_linestring(seg: Line<f32>, ls: &LineString<f32>) -> bool {
    for edge in ls.lines() {
        if segments_intersect(seg, edge) {
            return true;
        }
    }
    false
}

/// Compute the minimum distance from a point to a segment.
pub fn point_to_segment_distance(p: GeoPoint2D, seg: Line<f32>) -> f32 {
    // Point-to-segment distance: project p onto the line, clamp to segment.
    let ab = geo::Coord {
        x: seg.end.x - seg.start.x,
        y: seg.end.y - seg.start.y,
    };
    let ap = geo::Coord {
        x: p.x - seg.start.x,
        y: p.y - seg.start.y,
    };
    let ab_sq = ab.x * ab.x + ab.y * ab.y;
    if ab_sq < PRECISION {
        return distance(p, seg.start);
    }
    let t = ((ap.x * ab.x + ap.y * ab.y) / ab_sq).clamp(0.0, 1.0);
    let closest = geo::Coord {
        x: seg.start.x + t * ab.x,
        y: seg.start.y + t * ab.y,
    };
    distance(p, closest)
}

/// Compute the nearest point on a segment to a given point.
pub fn nearest_point_on_segment(p: GeoPoint2D, seg: Line<f32>) -> GeoPoint2D {
    use geo::Closest;
    match seg.closest_point(&geo::Point::from(p)) {
        Closest::SinglePoint(pt) | Closest::Intersection(pt) => pt.0,
        Closest::Indeterminate => seg.start, // degenerate segment
    }
}

/// Perpendicular-slab test — true iff the projection of `p` onto the
/// segment's direction lies within `[A, B]`, regardless of perpendicular
/// distance. Computes `vSeg = B - A`, returns `(p - A) · vSeg >= 0` AND
/// `(p - B) · vSeg <= 0` (equivalently, the two dot products have
/// opposite signs).
pub fn point_in_segment_slab(p: GeoPoint2D, seg: Line<f32>) -> bool {
    let vx = seg.end.x - seg.start.x;
    let vy = seg.end.y - seg.start.y;
    let da = (p.x - seg.start.x) * vx + (p.y - seg.start.y) * vy;
    let db = (p.x - seg.end.x) * vx + (p.y - seg.end.y) * vy;
    da * db <= 0.0
}

/// Length of a segment.
#[inline]
pub fn segment_length(seg: Line<f32>) -> f32 {
    distance(seg.start, seg.end)
}

// ─── Line2D / HalfLine2D ────────────────────────────────────────
//
// Three flavours of two-point primitive:
//   Line2D:     infinite line through points A and B
//   HalfLine2D: ray starting at A, going through B and beyond
//   Segment2D:  finite from A to B
//
// Line2D and HalfLine2D are lightly used (~20 files). We represent
// them as structs with two points, providing the key operations
// (eval, intersection) via methods.

/// An infinite line through two points.
#[derive(Debug, Clone, Copy)]
pub struct Line2D {
    pub a: GeoPoint2D,
    pub b: GeoPoint2D,
}

impl Line2D {
    pub fn new(a: GeoPoint2D, b: GeoPoint2D) -> Self {
        Self { a, b }
    }

    /// Direction vector (b - a).
    #[inline]
    pub fn direction(&self) -> Vec2D {
        pt(self.b.x - self.a.x, self.b.y - self.a.y)
    }

    /// Evaluate X given Y.
    ///
    /// Asserts the line is not horizontal (`dy != 0`); the explicit
    /// branch handles a vertical line, returning the constant `a.x`.
    pub fn eval_x(&self, y: f32) -> f32 {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        debug_assert!(dy.abs() >= PRECISION, "eval_x on horizontal line");
        if dx.abs() < PRECISION {
            self.a.x // vertical line — X constant along it
        } else {
            self.a.x + (y - self.a.y) * dx / dy
        }
    }

    /// Evaluate Y given X.
    ///
    /// Asserts the line is not vertical (`dx != 0`); the explicit
    /// branch handles a horizontal line, returning the constant `a.y`.
    pub fn eval_y(&self, x: f32) -> f32 {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        debug_assert!(dx.abs() >= PRECISION, "eval_y on vertical line");
        if dy.abs() < PRECISION {
            self.a.y // horizontal line — Y constant along it
        } else {
            self.a.y + (x - self.a.x) * dy / dx
        }
    }

    /// Test whether the **infinite** line crosses a finite segment.
    ///
    /// `vect1 = b - a`, then the segment endpoints must straddle the
    /// infinite line — i.e. `cross(vect1, seg.start - a)` and
    /// `cross(vect1, seg.end - a)` have opposite signs (zero on either
    /// side counts as a hit). A cheap segment-AABB-vs-line reject would
    /// shave a few cycles but isn't required for correctness, so it's
    /// omitted here.
    pub fn is_intersecting_segment(&self, seg: Line<f32>) -> bool {
        let v1 = self.direction();
        let det_a = cross(v1, pt(seg.start.x - self.a.x, seg.start.y - self.a.y));
        let det_b = cross(v1, pt(seg.end.x - self.a.x, seg.end.y - self.a.y));
        (det_a >= 0.0 && det_b <= 0.0) || (det_a <= 0.0 && det_b >= 0.0)
    }

    /// Test whether the infinite line crosses any edge of an open polyline.
    pub fn is_intersecting_polyline(&self, ls: &LineString<f32>) -> bool {
        for edge in ls.lines() {
            if self.is_intersecting_segment(edge) {
                return true;
            }
        }
        false
    }

    /// Test whether the infinite line crosses any (non-closing) edge of a
    /// polygon.
    ///
    /// **Quirk preserved:** the loop bound skips the closing edge from
    /// `vertex[N-1]` back to `vertex[0]`. This appears to be an upstream
    /// bug (the polygon overload reused the polyline's loop bound), but
    /// is preserved for behavioural parity, matching the same quirk in
    /// [`HalfLine2D::is_intersecting_polygon`].
    pub fn is_intersecting_polygon(&self, poly: &Polygon<f32>) -> bool {
        let edges: Vec<Line<f32>> = poly.exterior().lines().collect();
        // `LineString` is closed (first == last), so `.lines()` includes
        // the closing edge as its last element. Drop it to skip the
        // closing edge per the quirk above.
        let max = edges.len().saturating_sub(1);
        for edge in &edges[..max] {
            if self.is_intersecting_segment(*edge) {
                return true;
            }
        }
        false
    }
}

/// Geometric equality of infinite lines: direction vectors are colinear
/// AND the lines share a Y-intercept. For vertical lines we compare
/// `a.x` directly so two distinct vertical lines compare unequal rather
/// than tripping the [`Line2D::eval_y`] debug assert.
impl PartialEq for Line2D {
    fn eq(&self, other: &Self) -> bool {
        if cross(self.direction(), other.direction()) != 0.0 {
            return false;
        }
        let dx_self = self.b.x - self.a.x;
        let dx_other = other.b.x - other.a.x;
        if dx_self == 0.0 || dx_other == 0.0 {
            // Both vertical (cross == 0 ⇒ colinear directions); compare X.
            return self.a.x == other.a.x;
        }
        self.eval_y(0.0) == other.eval_y(0.0)
    }
}

/// A ray starting at A, passing through B.
#[derive(Debug, Clone, Copy)]
pub struct HalfLine2D {
    pub a: GeoPoint2D,
    pub b: GeoPoint2D,
}

/// Geometric equality of half-lines: same origin AND parallel direction,
/// regardless of how far past `b` each ray's defining "B" point sits.
/// Two rays sharing point A and going in the same direction compare equal
/// even if their defining "B" points are at different distances.
impl PartialEq for HalfLine2D {
    fn eq(&self, other: &Self) -> bool {
        self.a == other.a && cross(self.direction(), other.direction()) == 0.0
    }
}

impl HalfLine2D {
    pub fn new(a: GeoPoint2D, b: GeoPoint2D) -> Self {
        Self { a, b }
    }

    /// Direction vector (b - a).
    #[inline]
    pub fn direction(&self) -> Vec2D {
        pt(self.b.x - self.a.x, self.b.y - self.a.y)
    }

    /// Test if the half-line (ray from `a` through `b` and beyond)
    /// intersects the segment.
    ///
    /// Both segment endpoints must straddle the infinite line containing
    /// the half-line, then a side-of-line check from the half-line's
    /// origin ensures the crossing lies on the forward (b-side) ray
    /// rather than behind `a`.
    pub fn is_intersecting_segment(&self, seg: Line<f32>) -> bool {
        let v1 = self.direction();
        let a1a2 = pt(seg.start.x - self.a.x, seg.start.y - self.a.y);
        let a1b2 = pt(seg.end.x - self.a.x, seg.end.y - self.a.y);

        let det_v_a = cross(v1, a1a2);
        let det_v_b = cross(v1, a1b2);
        // Both segment endpoints must lie on opposite sides of the
        // infinite line containing the half-line.
        if !((det_v_a >= 0.0 && det_v_b <= 0.0) || (det_v_a <= 0.0 && det_v_b >= 0.0)) {
            return false;
        }
        // Side-of-half-line check from the half-line origin: ensures
        // the crossing lies on the forward (b-side) ray.
        let det_a_v = cross(a1a2, v1);
        let det_a_b = cross(a1a2, a1b2);
        if det_a_v > 0.0 {
            det_a_b >= 0.0
        } else {
            det_a_b <= 0.0
        }
    }

    /// Test if the half-line intersects any segment of an open polyline.
    pub fn is_intersecting_polyline(&self, polyline: &LineString<f32>) -> bool {
        for edge in polyline.lines() {
            if self.is_intersecting_segment(edge) {
                return true;
            }
        }
        false
    }

    /// Test if the half-line intersects any (non-closing) edge of a polygon.
    ///
    /// **Quirk preserved:** the loop bound skips the closing edge from
    /// `vertex[N-1]` back to `vertex[0]`. This appears to be an upstream
    /// bug (the polygon overload reused the polyline's loop bound), but
    /// is preserved for behavioural parity.
    pub fn is_intersecting_polygon(&self, poly: &Polygon<f32>) -> bool {
        let edges: Vec<Line<f32>> = poly.exterior().lines().collect();
        // `LineString` is closed (first == last), so `.lines()` includes
        // the closing edge as its last element. Drop it to skip the
        // closing edge per the quirk above.
        let max = edges.len().saturating_sub(1);
        for edge in &edges[..max] {
            if self.is_intersecting_segment(*edge) {
                return true;
            }
        }
        false
    }
}

// ─── Polygon operations ──────────────────────────────────────────

/// Test if a point is inside a polygon (boundary counts as inside).
#[inline]
pub fn polygon_contains_point(poly: &Polygon<f32>, p: GeoPoint2D) -> bool {
    poly.contains(&geo::Point::from(p)) || poly.exterior().intersects(&geo::Point::from(p))
}

/// Test whether a polygon (given as a vertex slice) intersects an
/// axis-aligned bounding box.
///   - empty polygon → false
///   - one-point polygon → point inside box
///   - any polygon edge intersects the box (including the closing edge)
///   - the box is fully contained in the polygon (top-left inside)
///
/// Used by the pathfinder state-change check to decide whether a
/// move-box of an actor is actually overlapped by a freshly-active
/// motion obstacle after the cheap bbox-vs-bbox pre-filter has already
/// passed.
pub fn polygon_vertices_intersect_bbox(vertices: &[GeoPoint2D], bbox: &BBox2D) -> bool {
    let rect = match bbox.0 {
        Some(r) => r,
        None => return false,
    };
    match vertices.len() {
        0 => false,
        1 => {
            let p = vertices[0];
            p.x >= rect.min().x && p.x <= rect.max().x && p.y >= rect.min().y && p.y <= rect.max().y
        }
        _ => {
            // Any vertex inside box
            for &p in vertices {
                if p.x >= rect.min().x
                    && p.x <= rect.max().x
                    && p.y >= rect.min().y
                    && p.y <= rect.max().y
                {
                    return true;
                }
            }
            // Any edge crosses box (including the closing edge
            // last → first).
            for i in 0..vertices.len() {
                let a = vertices[i];
                let b = vertices[(i + 1) % vertices.len()];
                if rect.intersects(&Line::new(a, b)) {
                    return true;
                }
            }
            // Box fully contained in polygon — top-left inside.
            // Build a polygon once only in this cold branch.
            let poly = Polygon::new(
                LineString::from(vertices.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()),
                vec![],
            );
            polygon_contains_point(&poly, pt(rect.min().x, rect.min().y))
        }
    }
}

/// Test if a polygon fully contains a segment (boundary counts as inside).
///
/// Compute the signed area of a polygon.
/// Positive = counter-clockwise, negative = clockwise.
pub fn polygon_signed_area(poly: &Polygon<f32>) -> f32 {
    use geo::Area;
    poly.signed_area()
}

/// Test if a polygon is clockwise.
pub fn polygon_is_clockwise(poly: &Polygon<f32>) -> bool {
    polygon_signed_area(poly) < 0.0
}

/// Test if a polygon is counter-clockwise.
pub fn polygon_is_counter_clockwise(poly: &Polygon<f32>) -> bool {
    polygon_signed_area(poly) > 0.0
}

/// Test if a polygon is convex.
///
/// Polygons with two or fewer distinct vertices are classified as
/// convex. `geo::is_convex::IsConvex` returns `false` for rings with
/// fewer than three distinct vertices, so the degenerate case is
/// handled here before falling through to the cross-product sweep.
pub fn polygon_is_convex(poly: &Polygon<f32>) -> bool {
    use geo::is_convex::IsConvex;
    // Closed `LineString` has the first vertex duplicated as the last,
    // so `len() <= 3` covers 0, 1, or 2 unique vertices.
    if poly.exterior().0.len() <= 3 {
        return true;
    }
    poly.exterior().is_convex()
}

// ─── Intersection result ─────────────────────────────────────────

/// Result of a geometric intersection calculation.
#[derive(Debug, Clone)]
pub enum Intersection2D {
    /// No intersection.
    None,
    /// Intersection at a single point.
    Point(GeoPoint2D),
    /// Overlap along a segment.
    Segment(Line<f32>),
}

/// Manual `PartialEq`: segment equality is direction-insensitive,
/// i.e. `(A,B) == (B,A)`. The derived `PartialEq` on `geo::Line<f32>`
/// would only match start/start and end/end, so for the `Segment`
/// variant we normalise endpoint order before comparing.
impl PartialEq for Intersection2D {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Intersection2D::None, Intersection2D::None) => true,
            (Intersection2D::Point(a), Intersection2D::Point(b)) => a == b,
            (Intersection2D::Segment(a), Intersection2D::Segment(b)) => {
                (a.start == b.start && a.end == b.end) || (a.start == b.end && a.end == b.start)
            }
            _ => false,
        }
    }
}

impl Intersection2D {
    pub fn is_none(&self) -> bool {
        matches!(self, Intersection2D::None)
    }

    pub fn point(&self) -> Option<GeoPoint2D> {
        match self {
            Intersection2D::Point(p) => Some(*p),
            _ => Option::None,
        }
    }
}

/// Compute the intersection of two line segments, if any.
/// Returns `Point` for a single crossing, `Segment` for collinear
/// overlap, `None` for no intersection.
pub fn segment_intersection(a: Line<f32>, b: Line<f32>) -> Intersection2D {
    use geo::line_intersection::{LineIntersection, line_intersection};
    match line_intersection(a, b) {
        Some(LineIntersection::SinglePoint { intersection, .. }) => {
            Intersection2D::Point(intersection)
        }
        Some(LineIntersection::Collinear { intersection }) => Intersection2D::Segment(intersection),
        Option::None => Intersection2D::None,
    }
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_basics() {
        let a = pt(1.0, 2.0);
        let b = pt(3.0, 4.0);
        assert_eq!(a.x, 1.0);
        assert_eq!(a.y, 2.0);
        assert!(points_near(a, a, 0.001));
        assert!(!points_near(a, b, 0.001));
    }

    #[test]
    fn vector_ops() {
        let a = pt(3.0, 4.0);
        assert!((length(a) - 5.0).abs() < 1e-6);
        let n = normalize(a);
        assert!((length(n) - 1.0).abs() < 1e-6);
        assert!((dot(a, pt(1.0, 0.0)) - 3.0).abs() < 1e-6);
        assert!((cross(pt(1.0, 0.0), pt(0.0, 1.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn bbox_hyperspace() {
        let b = BBox2D::new();
        assert!(!b.is_somewhere());
        assert!(!b.contains_point(pt(0.0, 0.0)));
    }

    #[test]
    fn bbox_from_point() {
        let b = BBox2D::from_point(pt(5.0, 10.0));
        assert!(b.is_somewhere());
        assert_eq!(b.x_min(), 5.0);
        assert_eq!(b.y_min(), 10.0);
        assert_eq!(b.width(), 0.0);
    }

    #[test]
    fn bbox_expand_points() {
        let mut b = BBox2D::new();
        b.expand_point(pt(1.0, 2.0));
        assert!(b.is_somewhere());
        assert_eq!(b.x_min(), 1.0);

        b.expand_point(pt(5.0, 8.0));
        assert_eq!(b.x_min(), 1.0);
        assert_eq!(b.x_max(), 5.0);
        assert_eq!(b.y_min(), 2.0);
        assert_eq!(b.y_max(), 8.0);

        b.expand_point(pt(-1.0, 3.0));
        assert_eq!(b.x_min(), -1.0);
    }

    #[test]
    fn bbox_expand_box() {
        let mut a = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        let b = BBox2D::from_coords(5.0, 5.0, 20.0, 20.0);
        a.expand_bbox(&b);
        assert_eq!(a.x_min(), 0.0);
        assert_eq!(a.x_max(), 20.0);
    }

    #[test]
    fn bbox_contains_point() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        assert!(b.contains_point(pt(5.0, 5.0)));
        assert!(b.contains_point(pt(0.0, 0.0))); // boundary
        assert!(b.contains_point(pt(10.0, 10.0))); // boundary
        assert!(!b.contains_point(pt(-1.0, 5.0)));
        assert!(!b.contains_point(pt(5.0, 11.0)));
    }

    #[test]
    fn bbox_is_boxed_point() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        // Half-open: top-left inclusive, bottom-right exclusive.
        assert!(b.is_boxed_point(pt(5.0, 5.0)));
        assert!(b.is_boxed_point(pt(0.0, 0.0))); // top-left edge included
        assert!(b.is_boxed_point(pt(0.0, 9.99)));
        assert!(!b.is_boxed_point(pt(10.0, 5.0))); // x == x_max excluded
        assert!(!b.is_boxed_point(pt(5.0, 10.0))); // y == y_max excluded
        assert!(!b.is_boxed_point(pt(10.0, 10.0))); // bottom-right corner excluded
        assert!(!b.is_boxed_point(pt(-1.0, 5.0)));
        // Hyperspace rejects everything.
        assert!(!BBox2D::new().is_boxed_point(pt(0.0, 0.0)));
    }

    #[test]
    fn bbox_is_on_boundary() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        // Edges
        assert!(b.is_on_boundary(pt(0.0, 5.0))); // left
        assert!(b.is_on_boundary(pt(10.0, 5.0))); // right
        assert!(b.is_on_boundary(pt(5.0, 0.0))); // top
        assert!(b.is_on_boundary(pt(5.0, 10.0))); // bottom
        // Corners
        assert!(b.is_on_boundary(pt(0.0, 0.0)));
        assert!(b.is_on_boundary(pt(10.0, 10.0)));
        // Strictly inside → false
        assert!(!b.is_on_boundary(pt(5.0, 5.0)));
        // Outside → false
        assert!(!b.is_on_boundary(pt(11.0, 5.0)));
        assert!(!b.is_on_boundary(pt(0.0, 11.0)));
        // Hyperspace
        assert!(!BBox2D::new().is_on_boundary(pt(0.0, 0.0)));
    }

    #[test]
    fn bbox_contains_bbox() {
        let outer = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        let inner = BBox2D::from_coords(2.0, 2.0, 8.0, 8.0);
        assert!(outer.contains_bbox(&inner));
        assert!(!inner.contains_bbox(&outer));
    }

    #[test]
    fn bbox_trivially_rejects_segment() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        // Segment fully left
        assert!(b.trivially_rejects_segment(segment(pt(-5.0, 5.0), pt(-1.0, 5.0))));
        // Segment crossing
        assert!(!b.trivially_rejects_segment(segment(pt(-1.0, 5.0), pt(5.0, 5.0))));
        // Hyperspace box rejects everything
        assert!(BBox2D::new().trivially_rejects_segment(segment(pt(0.0, 0.0), pt(1.0, 1.0))));
    }

    #[test]
    fn bbox_intersects_bbox() {
        let a = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        let b = BBox2D::from_coords(5.0, 5.0, 15.0, 15.0);
        let c = BBox2D::from_coords(20.0, 20.0, 30.0, 30.0);
        assert!(a.intersects_bbox(&b));
        assert!(!a.intersects_bbox(&c));
    }

    #[test]
    fn bbox_clip() {
        let a = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        let b = BBox2D::from_coords(5.0, 5.0, 15.0, 15.0);
        let c = a.clip_bbox(&b);
        assert!(c.is_somewhere());
        assert_eq!(c.x_min(), 5.0);
        assert_eq!(c.y_min(), 5.0);
        assert_eq!(c.x_max(), 10.0);
        assert_eq!(c.y_max(), 10.0);

        // Non-overlapping → hyperspace
        let d = BBox2D::from_coords(20.0, 20.0, 30.0, 30.0);
        assert!(!a.clip_bbox(&d).is_somewhere());
    }

    #[test]
    fn bbox_translate() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        let t = b.translated(pt(5.0, -3.0));
        assert_eq!(t.x_min(), 5.0);
        assert_eq!(t.y_min(), -3.0);
        assert_eq!(t.x_max(), 15.0);
        assert_eq!(t.y_max(), 7.0);
    }

    #[test]
    fn bbox_center() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 20.0);
        let c = b.center();
        assert_eq!(c.x, 5.0);
        assert_eq!(c.y, 10.0);
    }

    #[test]
    fn distance_between_points() {
        let d = distance(pt(0.0, 0.0), pt(3.0, 4.0));
        assert!((d - 5.0).abs() < 1e-6);
    }

    #[test]
    fn rotation() {
        let p = pt(1.0, 0.0);
        let r = rotate_around(p, pt(0.0, 0.0), std::f32::consts::FRAC_PI_2);
        assert!((r.x - 0.0).abs() < 1e-5);
        assert!((r.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn bbox_from_linestring() {
        let ls = LineString::from(vec![(0.0_f32, 0.0), (5.0, 3.0), (2.0, 7.0)]);
        let b = BBox2D::from(&ls);
        assert!(b.is_somewhere());
        assert_eq!(b.x_min(), 0.0);
        assert_eq!(b.x_max(), 5.0);
        assert_eq!(b.y_min(), 0.0);
        assert_eq!(b.y_max(), 7.0);
    }

    // ─── Segment ops ─────────────────────────────

    #[test]
    fn segment_segment_intersection_crossing() {
        let a = segment(pt(0.0, 0.0), pt(10.0, 10.0));
        let b = segment(pt(0.0, 10.0), pt(10.0, 0.0));
        assert!(segments_intersect(a, b));
        let ix = segment_intersection(a, b);
        let p = ix.point().unwrap();
        assert!((p.x - 5.0).abs() < 1e-4);
        assert!((p.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn segment_segment_no_intersection() {
        let a = segment(pt(0.0, 0.0), pt(1.0, 0.0));
        let b = segment(pt(0.0, 1.0), pt(1.0, 1.0));
        assert!(!segments_intersect(a, b));
        assert!(segment_intersection(a, b).is_none());
    }

    #[test]
    fn nearest_point_and_distance() {
        let seg = segment(pt(0.0, 0.0), pt(10.0, 0.0));
        // Point above midpoint
        let p = pt(5.0, 3.0);
        let nearest = nearest_point_on_segment(p, seg);
        assert!((nearest.x - 5.0).abs() < 1e-4);
        assert!((nearest.y - 0.0).abs() < 1e-4);
        let d = point_to_segment_distance(p, seg);
        assert!((d - 3.0).abs() < 1e-4);
    }

    #[test]
    fn nearest_point_clamped_to_endpoint() {
        let seg = segment(pt(0.0, 0.0), pt(10.0, 0.0));
        let p = pt(-5.0, 0.0);
        let nearest = nearest_point_on_segment(p, seg);
        assert!((nearest.x - 0.0).abs() < 1e-4);
    }

    #[test]
    fn segment_length_test() {
        let s = segment(pt(0.0, 0.0), pt(3.0, 4.0));
        assert!((segment_length(s) - 5.0).abs() < 1e-5);
    }

    // ─── Line2D ─────────────────────────────────

    #[test]
    fn line_eval_y() {
        // Line from (0,0) to (10,5) → slope 0.5
        let l = Line2D::new(pt(0.0, 0.0), pt(10.0, 5.0));
        assert!((l.eval_y(4.0) - 2.0).abs() < 1e-5);
        assert!((l.eval_y(0.0) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn line_eval_x() {
        let l = Line2D::new(pt(0.0, 0.0), pt(5.0, 10.0));
        assert!((l.eval_x(4.0) - 2.0).abs() < 1e-5);
    }

    // ─── Polygon ops ────────────────────────────

    #[test]
    fn polygon_contains() {
        let poly = Polygon::new(
            LineString::from(vec![
                (0.0_f32, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(polygon_contains_point(&poly, pt(5.0, 5.0)));
        assert!(!polygon_contains_point(&poly, pt(15.0, 5.0)));
    }

    #[test]
    fn polygon_orientation() {
        // CCW square
        let ccw = Polygon::new(
            LineString::from(vec![
                (0.0_f32, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(polygon_is_counter_clockwise(&ccw));
        assert!(!polygon_is_clockwise(&ccw));

        // CW square (reversed winding)
        let cw = Polygon::new(
            LineString::from(vec![
                (0.0_f32, 0.0),
                (0.0, 10.0),
                (10.0, 10.0),
                (10.0, 0.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(polygon_is_clockwise(&cw));
    }

    #[test]
    fn polygon_convexity() {
        // Convex square
        let square = Polygon::new(
            LineString::from(vec![
                (0.0_f32, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(polygon_is_convex(&square));

        // L-shape (concave)
        let l_shape = Polygon::new(
            LineString::from(vec![
                (0.0_f32, 0.0),
                (10.0, 0.0),
                (10.0, 5.0),
                (5.0, 5.0),
                (5.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(!polygon_is_convex(&l_shape));
    }

    // ─── Intersection result ────────────────────

    #[test]
    fn intersection_result_types() {
        let none = Intersection2D::None;
        assert!(none.is_none());
        assert!(none.point().is_none());

        let pt_ix = Intersection2D::Point(pt(3.0, 4.0));
        assert!(!pt_ix.is_none());
        assert_eq!(pt_ix.point().unwrap(), pt(3.0, 4.0));
    }

    // ─── BBox (from earlier) ────────────────────

    #[test]
    fn intersects_segment() {
        let b = BBox2D::from_coords(0.0, 0.0, 10.0, 10.0);
        // Crossing segment
        assert!(b.intersects_segment(segment(pt(-1.0, 5.0), pt(5.0, 5.0))));
        // Inside segment
        assert!(b.intersects_segment(segment(pt(2.0, 2.0), pt(8.0, 8.0))));
        // Outside segment
        assert!(!b.intersects_segment(segment(pt(20.0, 20.0), pt(30.0, 30.0))));
    }
}
