//! Runtime sprite-occlusion masks.
//!
//! A mask is a 2D screen-space bitmap attached to a building (or other
//! obstacle) that marks the pixels where the building covers the actor
//! layer.  When an actor walks behind the building, its sprite is
//! occluded: the covered pixels get replaced by the background.
//!
//! The raw mask data in the level file is a tiny RLE-packed bitmap; we
//! decode it here into a flat 1-byte-per-pixel buffer for fast per-pixel
//! lookups during render-time compositing.
//!
//! The polyline test (`is_applied_to_point_character`) decides whether
//! a character at (x, y) is occluded: it is occluded when the mask's
//! polyline `y` at that `x` is strictly greater than the character's
//! `y` (in screen coords, "lower on screen") — i.e. the character is
//! "above" the polyline and therefore behind the building.

use serde::{Deserialize, Serialize};

use crate::geo2d::{BBox2D, Point2D, pt};
use crate::level_data::{MASK_CHARACTER, MASK_OBSTACLE, MASK_PROJECTILE, MASK_VIEW, RawMask};

// ---------------------------------------------------------------------------
// MaskIndex — nominal newtype
// ---------------------------------------------------------------------------

/// Index into `FastFindGrid::level::masks` (sprite-occlusion masks).
/// Wraps [`nonmax::NonMaxU32`] so `Option<MaskIndex>` is 4 bytes via
/// niche optimization.
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
pub struct MaskIndex(pub nonmax::NonMaxU32);

impl MaskIndex {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}
impl From<MaskIndex> for u32 {
    #[inline]
    fn from(i: MaskIndex) -> u32 {
        i.0.get()
    }
}
impl From<MaskIndex> for usize {
    #[inline]
    fn from(i: MaskIndex) -> usize {
        i.0.get() as usize
    }
}
impl std::fmt::Display for MaskIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

/// Runtime form of a building/occlusion mask.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct RuntimeMask {
    /// Layer this mask belongs to.
    pub layer: u16,
    /// Bitfield of `MASK_CHARACTER | MASK_PROJECTILE | MASK_VIEW | MASK_OBSTACLE`.
    pub mask_type: u8,
    // Runtime active toggle (patches can flip it) lives in
    // [`FastFindGrid::mask_active`], keyed by mask index.
    /// Mask bounding box in world (map) coordinates.
    pub bbox: BBox2D,

    /// Polyline describing the character-masking silhouette (world coords).
    /// Sorted by increasing X.  Used by `is_applied_to_point_character` to
    /// decide whether an actor at a given position is behind the mask.
    pub character_polyline: Vec<Point2D>,

    /// Highest y (lower on screen) seen along the character polyline — used
    /// by `is_applied_to_box` for the wide-box shortcut.
    pub lower_y_for_mask: f32,

    /// Polyline for projectile masking (world coords).  Used by
    /// `is_applied_to_point_projectile` / `is_applied_to_point_3d` to
    /// mask flying entities (projectiles, flying humans).
    pub projectile_polyline: Vec<Point2D>,

    /// Indices into the engine's sight-obstacle list — populated only when
    /// `mask_type & MASK_OBSTACLE`.  Used by the 3D altitude check in
    /// `is_applied_to_point_3d` so a flying entity above the obstacle's
    /// top plane passes cleanly over the building.
    pub obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,

    /// Decoded bitmap width (= `bbox.width() as u16`).
    pub width: u16,
    /// Decoded bitmap height (= `bbox.height() as u16`).
    pub height: u16,
    /// Flat row-major bitmap, 1 byte per pixel: `1` = covered (building in
    /// front of actor), `0` = not covered.  `bitmap.len() == width * height`.
    pub bitmap: Vec<u8>,
}

impl RuntimeMask {
    /// Build a runtime mask from the raw level-file form.
    ///
    /// Returns `None` if the mask has no character polyline (only
    /// projectile / obstacle kinds) — we only use character masks for
    /// sprite occlusion so those can be skipped for now.
    pub fn from_raw(raw: &RawMask) -> Option<Self> {
        let (bbox_x, bbox_y) = raw.box_top_left;
        let (bbox_w, bbox_h) = raw.box_size;
        // Degenerate masks are skipped.
        if bbox_w <= 0 || bbox_h <= 0 {
            return None;
        }

        let bbox = BBox2D::from_coords(
            bbox_x as f32,
            bbox_y as f32,
            (bbox_x + bbox_w) as f32,
            (bbox_y + bbox_h) as f32,
        );

        // Decode the RLE bitmap up-front.  We always need it for sprite
        // compositing, and the on-disk size is tiny (a few KB per mask).
        let width = bbox_w as u16;
        let height = bbox_h as u16;
        let bitmap = decode_mask_bitmap(&raw.mask_data, width, height);

        // Character polyline: sort-of-present only when MASK_CHARACTER is set.
        // When absent we cannot apply to actors, so skip.
        let character_polyline: Vec<Point2D> = raw
            .character_polyline
            .as_ref()
            .map(|pts| pts.iter().map(|&(x, y)| pt(x as f32, y as f32)).collect())
            .unwrap_or_default();
        if character_polyline.is_empty() && (raw.mask_type & MASK_CHARACTER) != 0 {
            // Declared character mask but no polyline — skip, since the
            // polyline test would early-exit on empty anyway.
            return None;
        }

        let projectile_polyline: Vec<Point2D> = raw
            .projectile_polyline
            .as_ref()
            .map(|pts| pts.iter().map(|&(x, y)| pt(x as f32, y as f32)).collect())
            .unwrap_or_default();

        // Last-write-wins semantics for `lower_y_for_mask`: both the
        // character and projectile init passes write to the same scalar,
        // so the projectile polyline's max-Y wins whenever it is present
        // (mixed or projectile-only masks).  Character-only masks fall
        // back to the character polyline.
        let lower_y_source = if !projectile_polyline.is_empty() {
            &projectile_polyline
        } else {
            &character_polyline
        };
        let lower_y_for_mask = lower_y_source
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max);

        Some(Self {
            layer: raw.layer,
            mask_type: raw.mask_type,
            bbox,
            character_polyline,
            lower_y_for_mask,
            projectile_polyline,
            obstacle_indices: raw
                .obstacle_indices
                .iter()
                .filter_map(|&i| crate::sight_obstacle::SightObstacleIndex::new(u32::from(i)))
                .collect(),
            width,
            height,
            bitmap,
        })
    }

    /// Whether this mask can affect character sprites.
    #[inline]
    pub fn is_character(&self) -> bool {
        (self.mask_type & MASK_CHARACTER) != 0
    }

    /// Whether this mask can affect projectile sprites.
    #[inline]
    pub fn is_projectile(&self) -> bool {
        (self.mask_type & MASK_PROJECTILE) != 0
    }

    /// Whether this mask carries view-blocking data.
    #[inline]
    pub fn is_view(&self) -> bool {
        (self.mask_type & MASK_VIEW) != 0
    }

    /// Whether this mask is associated with 3D obstacles.
    #[inline]
    pub fn is_obstacle(&self) -> bool {
        (self.mask_type & MASK_OBSTACLE) != 0
    }

    /// Returns `true` when the mask's character polyline at `x = pt.x` has a
    /// `y` value strictly greater than `pt.y` — that is, the test point is
    /// "above" the polyline and therefore behind the building in screen
    /// coordinates (y grows downward, so "greater y" means "lower on screen").
    pub fn is_applied_to_point_character(&self, point: Point2D) -> bool {
        polyline_above_point(&self.character_polyline, point)
    }

    /// Same shape as `is_applied_to_point_character` but consults the
    /// projectile-masking polyline instead of the character one.
    pub fn is_applied_to_point_projectile(&self, point: Point2D) -> bool {
        polyline_above_point(&self.projectile_polyline, point)
    }

    /// Used for projectile and flying-human sprite occlusion.  First runs
    /// the 2D projectile-polyline test; if that misses and the mask carries
    /// `MASK_OBSTACLE`, walks the mask's `SightObstacle` list and returns
    /// `true` only when the test point sits **below** the relevant 3D plane
    /// of an obstacle whose ground polygon contains it (so a flying entity
    /// soaring above the building is left visible).
    ///
    /// `is_human` selects the bottom plane (humans look at the floor of
    /// the obstacle volume — they fall through it from above) versus the
    /// top plane (projectiles only see the obstacle when below its roof).
    pub fn is_applied_to_point_3d(
        &self,
        point: crate::coordinates::WorldPoint3D,
        is_human: bool,
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) -> bool {
        let point2d = pt(point.x, point.y);
        if self.is_applied_to_point_projectile(point2d) {
            return true;
        }
        if (self.mask_type & MASK_OBSTACLE) == 0 {
            return false;
        }
        for &obs_idx in &self.obstacle_indices {
            let Some(obs) = obstacles.get(usize::from(obs_idx)) else {
                continue;
            };
            if !obs.box_ground.contains_point(point2d) {
                continue;
            }
            if !obs.contains_point(point2d) {
                continue;
            }
            let plane_z = if is_human {
                obs.compute_bottom_z(point.x, point.y)
            } else {
                obs.compute_top_z(point.x, point.y) - 0.1
            };
            if plane_z > point.z {
                return true;
            }
        }
        false
    }
}

/// Shared "polyline-above-point" test used by both character and projectile
/// applications.  Returns true when the polyline's interpolated Y at
/// `point.x` is strictly greater than `point.y` (screen-space "above").
fn polyline_above_point(poly: &[Point2D], point: Point2D) -> bool {
    if poly.len() < 2 {
        return false;
    }

    let first_x = poly[0].x;
    let last_x = poly[poly.len() - 1].x;
    if point.x < first_x || point.x > last_x {
        return false;
    }

    // Walk the polyline until we find the segment whose X range contains point.x.
    // Starting from i=1, advance while `poly[i].x < point.x` so we land on
    // the first vertex whose x is >= point.x.
    let mut i = 1;
    while i < poly.len() && poly[i].x < point.x {
        i += 1;
    }
    if i >= poly.len() {
        return false;
    }

    let a = poly[i - 1];
    let b = poly[i];
    let dx = b.x - a.x;
    // Vertical segment: fall back to the higher endpoint (avoids dividing
    // by zero in the linear interpolation below).
    let y_on_seg = if dx.abs() < 1e-6 {
        a.y.max(b.y)
    } else {
        let t = (point.x - a.x) / dx;
        a.y + t * (b.y - a.y)
    };

    y_on_seg > point.y
}

/// Decode a mask's RLE-packed bitmap into a flat row-major `u8` buffer.
///
/// Output layout: one byte per pixel, `bitmap[y * width + x]` is `1` where
/// the mask is set (building covers actor), `0` otherwise.  The packed
/// format is:
///
/// * Each of `height` lines is prefixed by a length byte giving the count
///   of payload bytes that follow.
/// * Within a line, one or more (control, data...) records cover the full
///   row width in groups of 8 pixels at a time.
/// * The control byte encodes `(pattern_bytes << 3)` pixels in its low 7
///   bits and a "compressed" flag in its high bit (`0x80`).
/// * Compressed: one following data byte that is tiled as a cyclic 8-bit
///   pattern across `pattern_pixels`.
/// * Uncompressed: `pattern_bytes` following data bytes, each an MSB-first
///   8-pixel block.
pub fn decode_mask_bitmap(packed: &[u8], width: u16, height: u16) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut out = vec![0u8; w * h];

    let mut off = 0usize;
    for y in 0..h {
        if off >= packed.len() {
            break;
        }
        let line_len = packed[off] as usize;
        off += 1;
        let line_end = off.saturating_add(line_len).min(packed.len());

        let row_start = y * w;
        let mut x = 0usize;
        let mut pos = off;

        while x < w && pos < line_end {
            let control = packed[pos];
            pos += 1;
            let compressed = (control & 0x80) != 0;
            let pattern_bytes = (control & 0x7F) as usize;
            let pattern_pixels = pattern_bytes << 3;
            if pattern_pixels == 0 {
                continue;
            }

            if compressed {
                if pos >= line_end {
                    break;
                }
                let pattern = packed[pos];
                pos += 1;
                // Tile the 8-bit pattern across `pattern_pixels` pixels,
                // rotating an MSB-first bit mask starting at 0x80.
                for i in 0..pattern_pixels {
                    let px = x + i;
                    if px >= w {
                        break;
                    }
                    let bit = 0x80u8 >> (i & 7);
                    if pattern & bit != 0 {
                        out[row_start + px] = 1;
                    }
                }
                x += pattern_pixels;
            } else {
                // Raw 8-pixel blocks: one data byte per 8 pixels.
                for bi in 0..pattern_bytes {
                    if pos >= line_end {
                        break;
                    }
                    let byte = packed[pos];
                    pos += 1;
                    for i in 0..8 {
                        let px = x + bi * 8 + i;
                        if px >= w {
                            break;
                        }
                        let bit = 0x80u8 >> i;
                        if byte & bit != 0 {
                            out[row_start + px] = 1;
                        }
                    }
                }
                x += pattern_pixels;
            }
        }

        off = line_end;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_empty() {
        let bitmap = decode_mask_bitmap(&[], 8, 1);
        assert_eq!(bitmap, vec![0u8; 8]);
    }

    #[test]
    fn decode_compressed_all_set() {
        // Line length = 2, then control (compressed, 1 byte pattern -> 8 pixels), pattern 0xFF.
        let packed = [2u8, 0x81, 0xFF];
        let bitmap = decode_mask_bitmap(&packed, 8, 1);
        assert_eq!(bitmap, vec![1u8; 8]);
    }

    #[test]
    fn decode_compressed_alternating() {
        // 1 byte pattern covering 8 pixels: 0xAA = 10101010.
        let packed = [2u8, 0x81, 0xAA];
        let bitmap = decode_mask_bitmap(&packed, 8, 1);
        assert_eq!(bitmap, vec![1, 0, 1, 0, 1, 0, 1, 0]);
    }

    #[test]
    fn decode_uncompressed_two_bytes() {
        // Uncompressed control = 0x02 → 2 bytes covering 16 pixels.
        let packed = [3u8, 0x02, 0xF0, 0x0F];
        let bitmap = decode_mask_bitmap(&packed, 16, 1);
        assert_eq!(bitmap, vec![1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn point_character_test_basic() {
        // Polyline: (0,10) → (10,10).  Points with y < 10 are "above" and masked.
        let raw = RawMask {
            layer: 0,
            mask_type: MASK_CHARACTER,
            character_polyline: Some(vec![(0, 10), (10, 10)]),
            projectile_polyline: None,
            box_top_left: (0, 0),
            box_size: (16, 16),
            mask_data: vec![0u8; 16], // one zero-length byte per row
            obstacle_indices: vec![],
        };
        let mask = RuntimeMask::from_raw(&raw).unwrap();
        // Above polyline → masked.
        assert!(mask.is_applied_to_point_character(pt(5.0, 5.0)));
        // Below polyline → not masked.
        assert!(!mask.is_applied_to_point_character(pt(5.0, 15.0)));
        // Out of x-range → not masked.
        assert!(!mask.is_applied_to_point_character(pt(-1.0, 0.0)));
        assert!(!mask.is_applied_to_point_character(pt(20.0, 0.0)));
    }

    /// Build a flat-roof obstacle at z_top=10 / z_bottom=0 covering the
    /// 0..10 x 0..10 ground square — used by the 3D mask tests below.
    fn flat_obstacle_box() -> crate::sight_obstacle::SightObstacle {
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        let mut obs = SightObstacle::new(0, 0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();
        obs.top_plane_points = [[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [0.0, 10.0, 10.0]];
        obs.bottom_plane_points = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        obs
    }

    #[test]
    fn point_3d_projectile_polyline_short_circuits() {
        // Projectile polyline at y=10 with no obstacles — a point above
        // the polyline (y=5) is masked regardless of altitude.
        let mask = RuntimeMask {
            layer: 0,
            mask_type: MASK_PROJECTILE,
            bbox: BBox2D::from_coords(0.0, 0.0, 16.0, 16.0),
            character_polyline: vec![],
            lower_y_for_mask: f32::NEG_INFINITY,
            projectile_polyline: vec![pt(0.0, 10.0), pt(10.0, 10.0)],
            obstacle_indices: vec![],
            width: 16,
            height: 16,
            bitmap: vec![0u8; 16 * 16],
        };

        use crate::coordinates::WorldPoint3D;
        // Above polyline → masked, even at high altitude (no obstacle to
        // override the 2D test).
        assert!(mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: 1000.0
            },
            false,
            crate::sight_obstacle::ObstacleList::empty()
        ));
        // Below polyline → not masked.
        assert!(!mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 15.0,
                z: 0.0
            },
            false,
            crate::sight_obstacle::ObstacleList::empty()
        ));
    }

    #[test]
    fn point_3d_obstacle_altitude_check() {
        // Mask references one obstacle (top z=10) but has no projectile
        // polyline — entries are masked only when below the top plane.
        let mask = RuntimeMask {
            layer: 0,
            mask_type: MASK_OBSTACLE,
            bbox: BBox2D::from_coords(0.0, 0.0, 10.0, 10.0),
            character_polyline: vec![],
            lower_y_for_mask: f32::NEG_INFINITY,
            projectile_polyline: vec![],
            obstacle_indices: vec![crate::sight_obstacle::SightObstacleIndex::new(0).unwrap()],
            width: 0,
            height: 0,
            bitmap: vec![],
        };
        let obstacles = vec![flat_obstacle_box()];

        use crate::coordinates::WorldPoint3D;
        // Projectile at z=5 inside ground polygon, below z_top=10 → masked.
        assert!(mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: 5.0
            },
            false,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
        // Projectile soaring at z=20 above z_top=10 → not masked.
        assert!(!mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: 20.0
            },
            false,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
        // Projectile outside the obstacle ground polygon → not masked.
        assert!(!mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 50.0,
                y: 5.0,
                z: 5.0
            },
            false,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
        // Human at z=5 above z_bottom=0 → not masked (human falls through
        // the floor when above it; only masked when below the bottom plane).
        assert!(!mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: 5.0
            },
            true,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
        // Human at z=-5 below the obstacle floor → masked.
        assert!(mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: -5.0
            },
            true,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
    }

    #[test]
    fn point_3d_skips_obstacle_check_without_flag() {
        // Same mask as above but without MASK_OBSTACLE — even though
        // obstacle_indices is populated, the altitude check is skipped.
        let mask = RuntimeMask {
            layer: 0,
            mask_type: MASK_PROJECTILE,
            bbox: BBox2D::from_coords(0.0, 0.0, 10.0, 10.0),
            character_polyline: vec![],
            lower_y_for_mask: f32::NEG_INFINITY,
            projectile_polyline: vec![],
            obstacle_indices: vec![crate::sight_obstacle::SightObstacleIndex::new(0).unwrap()],
            width: 0,
            height: 0,
            bitmap: vec![],
        };
        let obstacles = vec![flat_obstacle_box()];
        use crate::coordinates::WorldPoint3D;
        assert!(!mask.is_applied_to_point_3d(
            WorldPoint3D {
                x: 5.0,
                y: 5.0,
                z: 5.0
            },
            false,
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacles)
        ));
    }
}
