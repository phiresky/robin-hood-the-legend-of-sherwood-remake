//! Full material-sector registry used by footstep / sprite-material lookup.
//!
//! Material sectors are SECTOR_SOUND polygons queried spatially: each
//! hit's material is returned when the probe point lies inside the
//! polygon, falling back to a per-map default material when no sector
//! contains the point.
//!
//! The Rust `FastFindGrid` doesn't yet index material sectors (they're unused
//! by mouse picking / motion / sight), so we keep a standalone list here.
//! This is a superset of [`crate::water_zones::WaterZones`], which keeps just
//! the water / hole polygons for projectile splash detection.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::GameMaterial;
use crate::geo2d::{BBox2D, pt};
use crate::level_data::RawMaterialSector;

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MaterialSector {
    pub points: Vec<MapPoint>,
    pub bounding_box: BBox2D,
    pub material: GameMaterial,
}

impl MaterialSector {
    /// Convert one `RawMaterialSector` into a `MaterialSector`, applying
    /// the material-code substitution: codes >= `N_MATERIALS` (9) fall
    /// back to the supplied default material.  Returns `None` for
    /// degenerate polygons (< 3 vertices) — they can never contain a
    /// point.
    pub fn from_raw(r: &RawMaterialSector, default_material: GameMaterial) -> Option<Self> {
        const N_MATERIALS: u32 = 9;
        const LIGHT_SHADOW: u32 = 10;
        if r.polygon.points.len() < 3 {
            return None;
        }
        let points: Vec<MapPoint> = r
            .polygon
            .points
            .iter()
            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
            .collect();
        let mut bbox = BBox2D::new();
        for &p in &points {
            bbox.expand_point(p.to_geo());
        }
        let code = r.material as u32;
        debug_assert!(
            code != LIGHT_SHADOW,
            "MATERIAL_LIGHT_SHADOW must not appear in CHUNK_MATERIAL"
        );
        let material = if code >= N_MATERIALS {
            default_material
        } else {
            GameMaterial::from_u32(code)
        };
        Some(MaterialSector {
            points,
            bounding_box: bbox,
            material,
        })
    }

    /// Ray-casting point-in-polygon test — same implementation as
    /// [`crate::water_zones::WaterZone::contains`].
    pub fn contains(&self, p: MapPoint) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        if !self.bounding_box.contains_point(p.to_geo()) {
            return false;
        }
        let mut inside = false;
        let n = self.points.len();
        let mut j = n - 1;
        for i in 0..n {
            let vi = self.points[i];
            let vj = self.points[j];
            if (vi.y > p.y) != (vj.y > p.y) {
                let x_intersect = (vj.x - vi.x) * (p.y - vi.y) / (vj.y - vi.y) + vi.x;
                if p.x < x_intersect {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Approximate distance from `point` to this polygon's boundary
    /// under the 1-norm (|dx| + |dy|), with Y pre-stretched by
    /// `inverse_aspect_ratio` so iso-space distances work out.
    ///
    /// The algorithm:
    /// 1. min over the 1-norm distance to each vertex
    /// 2. min over the axis-aligned distance to each edge, where an
    ///    edge is classified "horizontalish" vs "verticalish" by
    ///    comparing dX to ±dY, and distance is measured along the
    ///    minor axis of the edge (Y for horizontalish, X for
    ///    verticalish).  Edges whose axis-aligned strip doesn't cover
    ///    the point's projection are skipped.
    ///
    /// Returns a large sentinel value (`f32::MAX / 2`) for empty
    /// polygons.
    pub fn approximate_distance_to_boundary(
        &self,
        point: MapPoint,
        inverse_aspect_ratio: f32,
    ) -> f32 {
        let mut min_distance = f32::MAX / 2.0;

        let n = self.points.len();
        if n == 0 {
            return min_distance;
        }

        let modified_point = pt(point.x, point.y * inverse_aspect_ratio);

        // (I) check all points (vertex distance, 1-norm)
        for &v in &self.points {
            let va = pt(v.x, v.y * inverse_aspect_ratio);
            let distance = (modified_point.x - va.x).abs() + (modified_point.y - va.y).abs();
            if distance < min_distance {
                min_distance = distance;
            }
        }

        // (II) check all segments
        let mut pt_a = {
            let v = self.points[n - 1];
            pt(v.x, v.y * inverse_aspect_ratio)
        };
        for i in 0..n {
            let pt_b = {
                let v = self.points[i];
                pt(v.x, v.y * inverse_aspect_ratio)
            };
            let b_minus_a = pt(pt_b.x - pt_a.x, pt_b.y - pt_a.y);

            // Classify edge orientation:
            //   dX > dY ? horizontalish = (dX > -dY) : horizontalish = (dX < -dY)
            let horizontalish = if b_minus_a.x > b_minus_a.y {
                b_minus_a.x > -b_minus_a.y
            } else {
                b_minus_a.x < -b_minus_a.y
            };

            let x_min = pt_a.x.min(pt_b.x);
            let x_max = pt_a.x.max(pt_b.x);
            let y_min = pt_a.y.min(pt_b.y);
            let y_max = pt_a.y.max(pt_b.y);

            let mut distance;
            if horizontalish {
                if modified_point.x <= x_min
                    || modified_point.x >= x_max
                    || modified_point.y <= y_min - min_distance
                    || modified_point.y >= y_max + min_distance
                {
                    distance = f32::MAX;
                } else {
                    let ratio = (modified_point.x - pt_a.x) / b_minus_a.x;
                    distance = modified_point.y - (pt_a.y + ratio * b_minus_a.y);
                }
            } else if modified_point.y <= y_min
                || modified_point.y >= y_max
                || modified_point.x <= x_min - min_distance
                || modified_point.x >= x_max + min_distance
            {
                distance = f32::MAX;
            } else {
                let ratio = (modified_point.y - pt_a.y) / b_minus_a.y;
                distance = modified_point.x - (pt_a.x + ratio * b_minus_a.x);
            }

            if distance < 0.0 {
                distance = -distance;
            }
            if distance < min_distance {
                min_distance = distance;
            }

            pt_a = pt_b;
        }

        min_distance
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MaterialSectors {
    pub sectors: Vec<MaterialSector>,
    pub default_material: GameMaterial,
}

impl MaterialSectors {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from the proto material-sector list + the map's default material
    /// (from `CHUNK_MISC`).  Polygons with fewer than 3 vertices are skipped
    /// — they can never contain a point.
    pub fn build_from_raw(raw: &[RawMaterialSector], default_material_code: u32) -> Self {
        let default_material = GameMaterial::from_u32(default_material_code);
        let sectors = raw
            .iter()
            .filter_map(|r| MaterialSector::from_raw(r, default_material))
            .collect();
        Self {
            sectors,
            default_material,
        }
    }

    pub fn clear(&mut self) {
        self.sectors.clear();
        self.default_material = GameMaterial::default();
    }

    /// Material at the given map point — first SECTOR_SOUND polygon that
    /// contains the point wins, falling back to `default_material`.
    pub fn material_at(&self, point: MapPoint) -> GameMaterial {
        for s in &self.sectors {
            if s.contains(point) {
                return s.material;
            }
        }
        self.default_material
    }

    /// First sector containing `point`, if any.  Same first-hit scan
    /// as `material_at` but returns the sector itself so callers can
    /// query boundary distance etc.
    pub fn containing_sector(&self, point: MapPoint) -> Option<&MaterialSector> {
        self.sectors.iter().find(|s| s.contains(point))
    }

    /// Material at `point`, optionally constrained to a landing
    /// obstacle's sub-sector list.
    ///
    /// * `obstacle == None`: scan the global SECTOR_SOUND list,
    ///   falling back to `default_material`.  Same as
    ///   [`Self::material_at`].
    /// * `obstacle == Some(o)`: iterate the obstacle's
    ///   `material_sectors` — first containing sub-sector wins.  When
    ///   no sub-sector contains the point, fall back to the obstacle's
    ///   own overall material.  Note: the global SECTOR_SOUND list is
    ///   NOT consulted in this branch — projectiles landing on a
    ///   non-None obstacle are bound to that obstacle's material space.
    pub fn material_at_with_obstacle(
        &self,
        obstacle: Option<&crate::sight_obstacle::SightObstacle>,
        point: MapPoint,
    ) -> GameMaterial {
        match obstacle {
            None => self.material_at(point),
            Some(obs) => {
                for s in &obs.material_sectors {
                    if s.contains(point) {
                        return s.material;
                    }
                }
                GameMaterial::from_u32(obs.material as u32)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level_data::SectorPolygon;

    fn square(material: u8, min: i16, max: i16) -> RawMaterialSector {
        RawMaterialSector {
            material,
            polygon: SectorPolygon {
                points: vec![(min, min), (max, min), (max, max), (min, max)],
            },
        }
    }

    #[test]
    fn polygon_hit_wins_over_default() {
        let raw = vec![square(2 /* Stone */, 0, 10)];
        let ms = MaterialSectors::build_from_raw(&raw, 3 /* Grass */);
        assert_eq!(ms.material_at(MapPoint::new(5.0, 5.0)), GameMaterial::Stone);
    }

    #[test]
    fn fallback_to_default_material() {
        let raw = vec![square(2 /* Stone */, 0, 10)];
        let ms = MaterialSectors::build_from_raw(&raw, 3 /* Grass */);
        assert_eq!(
            ms.material_at(MapPoint::new(50.0, 50.0)),
            GameMaterial::Grass
        );
    }

    #[test]
    fn first_polygon_wins_on_overlap() {
        let raw = vec![square(2 /* Stone */, 0, 10), square(1 /* Wood */, 5, 15)];
        let ms = MaterialSectors::build_from_raw(&raw, 0);
        // (7,7) is inside both; the scan returns the first hit (Stone).
        assert_eq!(ms.material_at(MapPoint::new(7.0, 7.0)), GameMaterial::Stone);
    }

    #[test]
    fn distance_to_boundary_centre_of_square() {
        // 10×10 square; centre → 5 away from every edge under 1-norm
        // with inverse_aspect_ratio = 1.0.
        let raw = vec![square(7 /* Water */, 0, 10)];
        let ms = MaterialSectors::build_from_raw(&raw, 0);
        let s = &ms.sectors[0];
        let d = s.approximate_distance_to_boundary(MapPoint::new(5.0, 5.0), 1.0);
        assert!((d - 5.0).abs() < 1e-3, "expected ~5.0, got {d}");
    }

    #[test]
    fn distance_to_boundary_respects_inverse_aspect_ratio() {
        // Under Y-stretch of 2.0, a point at (5, 2) in a 10×10 square
        // maps to (5, 4) in iso-space; nearest edge under 1-norm is
        // the top edge y=0 (after stretch) at distance 4, not the
        // left/right edges at distance 5.
        let raw = vec![square(7 /* Water */, 0, 10)];
        let ms = MaterialSectors::build_from_raw(&raw, 0);
        let s = &ms.sectors[0];
        let d = s.approximate_distance_to_boundary(MapPoint::new(5.0, 2.0), 2.0);
        assert!((d - 4.0).abs() < 1e-3, "expected ~4.0, got {d}");
    }

    #[test]
    fn distance_to_boundary_empty_polygon_returns_sentinel() {
        let s = MaterialSector {
            points: vec![],
            bounding_box: BBox2D::new(),
            material: GameMaterial::Water,
        };
        let d = s.approximate_distance_to_boundary(MapPoint::new(0.0, 0.0), 1.0);
        assert!(d > 1e30, "empty polygon must return a large sentinel");
    }

    fn make_obstacle_with_sectors(
        material: u8,
        sub_sectors: Vec<MaterialSector>,
    ) -> crate::sight_obstacle::SightObstacle {
        let mut obs = crate::sight_obstacle::SightObstacle::new(0, 0);
        obs.material = material;
        obs.material_sectors = sub_sectors;
        obs
    }

    #[test]
    fn obstacle_sub_sector_hit_wins_over_overall_material() {
        // When an obstacle's sub-sector contains the impact point,
        // return the sub-sector's material — not the obstacle's overall.
        let raw = vec![square(0 /* Ground */, 100, 200)];
        let ms = MaterialSectors::build_from_raw(&raw, 0);
        let inlay =
            MaterialSector::from_raw(&square(2 /* Stone */, 0, 10), GameMaterial::default())
                .unwrap();
        let obs = make_obstacle_with_sectors(1 /* Wood */, vec![inlay]);
        // (5,5) hits the Stone inlay → Stone, not the obstacle's Wood.
        assert_eq!(
            ms.material_at_with_obstacle(Some(&obs), MapPoint::new(5.0, 5.0)),
            GameMaterial::Stone
        );
    }

    #[test]
    fn obstacle_fallback_to_obstacle_material() {
        // No sub-sector contains the point → return the obstacle's
        // overall material.  Crucially, this branch does NOT fall
        // through to the global SECTOR_SOUND list.
        let raw = vec![square(2 /* Stone */, 0, 200)];
        let ms = MaterialSectors::build_from_raw(&raw, 3 /* Grass */);
        let obs = make_obstacle_with_sectors(1 /* Wood */, vec![]);
        // (50,50) is inside the global Stone sector but the obstacle
        // path bypasses it — must return the obstacle's Wood material.
        assert_eq!(
            ms.material_at_with_obstacle(Some(&obs), MapPoint::new(50.0, 50.0)),
            GameMaterial::Wood
        );
    }

    #[test]
    fn obstacle_none_falls_through_to_global_scan() {
        // None obstacle → scan global SECTOR_SOUND polygons +
        // default fallback.
        let raw = vec![square(2 /* Stone */, 0, 10)];
        let ms = MaterialSectors::build_from_raw(&raw, 3 /* Grass */);
        assert_eq!(
            ms.material_at_with_obstacle(None, MapPoint::new(5.0, 5.0)),
            GameMaterial::Stone
        );
        assert_eq!(
            ms.material_at_with_obstacle(None, MapPoint::new(50.0, 50.0)),
            GameMaterial::Grass
        );
    }
}
