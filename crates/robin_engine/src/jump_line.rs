//! Runtime jump-line data for table swordfights and line jumps.
//!
//! A jump line is a segment that sits on the edge of a motion-area
//! sector and pairs with another jump line on the "other side" of a
//! jump gap.  Actors can cross a pair of jump lines in two ways:
//!
//! * **Line jump** — run to the source line, execute a jump sequence
//!   across the paired line, then continue to the clicked destination.
//! * **Table swordfight** — stand on your side's line and swordfight
//!   the enemy standing on theirs.  The swordstrike reaches across
//!   the gap without anyone needing to cross.
//!
//! Each line stores its two endpoints (`point_a`, `point_b`) as 2D
//! positions plus elevation (`z_a`, `z_b`), a reference back to its
//! own home sector (`sector_index`), and a reference to the paired
//! line on the other side (`associated_line_index`).
//!
//! The home-sector pointer is set during post-processing by the
//! slightly confusing rule that each line lives in its *paired*
//! line's jump-zone's sector (the jump-zone is a polygon representing
//! the airspace on the destination side).

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, MapVec};

// ---------------------------------------------------------------------------
// JumpLineIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `FastFindGrid::jump_lines`.  Wraps [`nonmax::NonMaxU32`]
/// so `Option<JumpLineIndex>` is 4 bytes via niche optimization.
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
pub struct JumpLineIndex(pub nonmax::NonMaxU32);

crate::bitcode_adapters::impl_native_bitcode_index!(JumpLineIndex, u32);

impl JumpLineIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<JumpLineIndex> for u32 {
    #[inline]
    fn from(i: JumpLineIndex) -> u32 {
        i.0.get()
    }
}
impl From<JumpLineIndex> for usize {
    #[inline]
    fn from(i: JumpLineIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for JumpLineIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

/// A jump line with 3D endpoints and paired-line / sector metadata.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct JumpLine {
    /// Map-space position of endpoint A.
    pub point_a: MapPoint,
    /// Map-space position of endpoint B.
    pub point_b: MapPoint,
    /// Elevation at point A.
    pub z_a: f32,
    /// Elevation at point B.
    pub z_b: f32,
    /// Index into `FastFindGrid::jump_lines` of the paired line on
    /// the far side of the jump.  `None` during construction, set to
    /// `Some(idx)` in post-processing.
    pub associated_line_index: Option<u32>,
    /// Index into `FastFindGrid::sectors` of this line's home
    /// sector.  Populated from the paired line's jump-zone's sector
    /// during post-processing.
    pub sector_index: Option<crate::fast_find_grid::SectorIndex>,
    /// Layer this line belongs to (copied from the paired line's
    /// jump zone's layer).
    pub layer: u16,
    /// Whether this pair is forced-long-jump.  Only the line-jump
    /// path reads this; the table-swordfight branch ignores it.
    pub long_jump_forced: bool,
    /// Whether landing on this line requires a helper (copied from
    /// the line's jump zone's `helper_needed` flag at load time).
    /// The cursor code reads the *associated* (destination) line's
    /// flag to decide whether to show a ghost titbit hint at the
    /// midpoint.
    pub helper_needed: bool,
}

impl JumpLine {
    /// Construct from raw proto data.  `jump_zone_index` is carried
    /// through in the caller since the paired-line / sector linkage
    /// is resolved after both lines of a pair are loaded.
    pub fn new(point_a: MapPoint, point_b: MapPoint, z_a: f32, z_b: f32) -> Self {
        Self {
            point_a,
            point_b,
            z_a,
            z_b,
            associated_line_index: None,
            sector_index: None,
            layer: 0,
            long_jump_forced: false,
            helper_needed: false,
        }
    }

    /// Vector from A to B.
    pub fn vector(&self) -> MapVec {
        self.point_b - self.point_a
    }

    /// Squared length of the A→B segment.
    pub fn square_norm(&self) -> f32 {
        let v = self.vector();
        v.x * v.x + v.y * v.y
    }

    /// Length of the A→B segment.
    pub fn norm(&self) -> f32 {
        self.square_norm().sqrt()
    }

    /// Midpoint of the line in 2D.
    pub fn get_middle_point(&self) -> MapPoint {
        MapPoint::new(
            0.5 * (self.point_a.x + self.point_b.x),
            0.5 * (self.point_a.y + self.point_b.y),
        )
    }

    /// Nearest-point parameter `t ∈ [0, 1]` on the A→B segment from
    /// `pt_test`.  Clamps to the A or B endpoint when the projection
    /// falls outside the segment.
    pub fn compute_nearest_point_param(&self, pt_test: MapPoint) -> f32 {
        let v = self.vector();
        // if ((pt - B) · v >= 0) return 1
        let dx_b = pt_test.x - self.point_b.x;
        let dy_b = pt_test.y - self.point_b.y;
        if dx_b * v.x + dy_b * v.y >= 0.0 {
            return 1.0;
        }
        let dx_a = pt_test.x - self.point_a.x;
        let dy_a = pt_test.y - self.point_a.y;
        let dot = dx_a * v.x + dy_a * v.y;
        if dot <= 0.0 {
            return 0.0;
        }
        let sq = self.square_norm();
        if sq < f32::EPSILON { 0.0 } else { dot / sq }
    }

    /// Distance from `pt_test` to the A→B segment: the perpendicular
    /// distance when the projection falls inside the segment,
    /// otherwise the distance to the nearest endpoint.
    pub fn compute_distance(&self, pt_test: MapPoint) -> f32 {
        let t = self.compute_nearest_point_param(pt_test);
        let v = self.vector();
        let nearest = self.point_a + v.scale(t);
        let dx = pt_test.x - nearest.x;
        let dy = pt_test.y - nearest.y;
        (dx * dx + dy * dy).sqrt()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;

    fn line(ax: f32, ay: f32, bx: f32, by: f32) -> JumpLine {
        JumpLine::new(map_pt(ax, ay), map_pt(bx, by), 0.0, 0.0)
    }

    #[test]
    fn nearest_point_projection_inside() {
        // A = (0,0), B = (10,0). Point (5, 3) projects to (5, 0).
        let jl = line(0.0, 0.0, 10.0, 0.0);
        let t = jl.compute_nearest_point_param(map_pt(5.0, 3.0));
        assert!((t - 0.5).abs() < 1e-4);
    }

    #[test]
    fn nearest_point_clamps_to_a() {
        let jl = line(0.0, 0.0, 10.0, 0.0);
        let t = jl.compute_nearest_point_param(map_pt(-5.0, 2.0));
        assert_eq!(t, 0.0);
    }

    #[test]
    fn nearest_point_clamps_to_b() {
        let jl = line(0.0, 0.0, 10.0, 0.0);
        let t = jl.compute_nearest_point_param(map_pt(15.0, 2.0));
        assert_eq!(t, 1.0);
    }

    #[test]
    fn distance_perpendicular() {
        // Inside the segment, perpendicular distance = 3.
        let jl = line(0.0, 0.0, 10.0, 0.0);
        let d = jl.compute_distance(map_pt(5.0, 3.0));
        assert!((d - 3.0).abs() < 1e-4);
    }

    #[test]
    fn distance_to_endpoint() {
        // Past B, distance from (15, 2) to (10, 0) = sqrt(25+4).
        let jl = line(0.0, 0.0, 10.0, 0.0);
        let d = jl.compute_distance(map_pt(15.0, 2.0));
        let expected = (29.0_f32).sqrt();
        assert!((d - expected).abs() < 1e-4);
    }

    #[test]
    fn vector_and_norm() {
        let jl = line(1.0, 2.0, 4.0, 6.0);
        let v = jl.vector();
        assert_eq!(v.x, 3.0);
        assert_eq!(v.y, 4.0);
        assert!((jl.norm() - 5.0).abs() < 1e-5);
    }

    #[test]
    fn middle_point() {
        let jl = line(0.0, 0.0, 10.0, 20.0);
        let m = jl.get_middle_point();
        assert_eq!(m.x, 5.0);
        assert_eq!(m.y, 10.0);
    }
}
