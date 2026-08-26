//! Water / hole zone detection for projectile splashes.
//!
//! Water and hole zones are material sectors whose material value is
//! `Water` or `Hole`. The detection logic scans such sectors at an
//! impact point and returns the material if the point is inside the
//! polygon (AABB + ray-casting test).
//!
//! Since `FastFindGrid` doesn't currently index material sectors (they
//! are not needed for mouse picking, motion, etc.), we keep a separate
//! lightweight list of just the water/hole polygons — that's all the
//! no-obstacle landing branch needs.

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapBBox, MapPoint};
use crate::level_data::RawMaterialSector;
use crate::sound_cache::Material;

/// A single water or hole polygon loaded from the proto material chunk.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct WaterZone {
    pub points: Vec<MapPoint>,
    pub bounding_box: MapBBox,
    /// Either [`Material::Water`] or [`Material::Hole`].
    pub material: Material,
}

impl WaterZone {
    /// Point-in-polygon test — AABB reject, then ray casting.
    pub fn contains(&self, p: MapPoint) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        if !self.bounding_box.contains_point(p) {
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
}

/// All water and hole zones on the current level.
///
/// Populated from [`robin_assets::level_loader::ProtoData::material_sectors`] at
/// level-load time. Empty before any level is loaded.
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
pub struct WaterZones {
    pub zones: Vec<WaterZone>,
}

/// Material lookup result tied to the exact sector polygon that produced it.
/// Original keeps the `RHSectorMaterial*` returned by `DetermineWaterHole` and
/// passes that same polygon to `AddTrajectoryFallIntoHole`.
#[derive(Debug, Clone, Copy)]
pub struct WaterHoleResolution<'a> {
    pub material: Material,
    pub sector_points: Option<&'a [MapPoint]>,
}

impl WaterZones {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from the proto material-sector list, keeping only water and
    /// hole entries. Polygons with fewer than 3 vertices are skipped
    /// since they can never contain a point.
    pub fn build_from_raw(raw: &[RawMaterialSector]) -> Self {
        let mut zones = Vec::new();
        for r in raw {
            // Material codes: 5 = WATER, 8 = HOLE. Must match
            // `sound_cache::material_from_u8`.
            let material = match r.material {
                5 => Material::Water,
                8 => Material::Hole,
                _ => continue,
            };
            if r.polygon.points.len() < 3 {
                continue;
            }
            let points: Vec<MapPoint> = r
                .polygon
                .points
                .iter()
                .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                .collect();
            let mut bbox = MapBBox::new();
            for &p in &points {
                bbox.expand_point(p);
            }
            zones.push(WaterZone {
                points,
                bounding_box: bbox,
                material,
            });
        }
        Self { zones }
    }

    pub fn clear(&mut self) {
        self.zones.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Return `Some(Water)` or `Some(Hole)` if the map-space point is
    /// inside a water or hole zone, else `None`.
    ///
    /// This is the no-obstacle branch — it just iterates the
    /// water/hole-only list, which is typically small (levels have at
    /// most a handful of water polygons), instead of looking up
    /// candidate sectors via the spatial grid.
    ///
    /// Use [`determine_water_hole_with_obstacle`] when the projectile's
    /// landing obstacle is known — the obstacle-anchored variant covers
    /// the cases of water lakes modelled as obstacles and holes carved
    /// into a roof.
    pub fn determine_water_hole(&self, point: MapPoint) -> Option<Material> {
        for z in &self.zones {
            if z.contains(point) {
                return Some(z.material);
            }
        }
        None
    }

    /// True iff `landing` (map / screen space) is inside a hole
    /// polygon. Used by the fall-into-hole trajectory which
    /// unconditionally marks the projectile as disappearing whenever
    /// the material at the landing is a hole.
    pub fn landing_is_in_hole(&self, landing: MapPoint) -> bool {
        self.zones
            .iter()
            .any(|z| matches!(z.material, Material::Hole) && z.contains(landing))
    }

    /// Extend a line in 2D from `entry` through `landing` and return the
    /// first intersection with a hole polygon edge past `landing`.
    /// Used to slide a landed projectile visually into the hole's far
    /// edge before it disappears, rather than stopping at the hole's
    /// near lip.
    ///
    /// `entry` is the trajectory's penultimate point and `landing` is
    /// the terminal (inside-the-hole) point.  Both are in screen /
    /// map-space (`y = pos.y - pos.z`).  The extension searches only
    /// the polygon that contains `landing`; returns `None` if
    /// `landing` is not inside any hole or no forward edge
    /// intersection exists.
    ///
    /// Selection criterion: the candidate edge intersection must have
    /// `isec.y > landing.y` (strictly greater in screen-Y), and the
    /// winner is the one with the smallest `isec.y` among those. This
    /// is intentionally screen-Y–anchored rather than
    /// trajectory-aligned because projectiles visually "fly into" the
    /// screen along +Y in isometric view.
    pub fn find_hole_far_exit(&self, entry: MapPoint, landing: MapPoint) -> Option<MapPoint> {
        // Find the hole polygon that contains the landing point.
        let hole = self
            .zones
            .iter()
            .find(|z| matches!(z.material, Material::Hole) && z.contains(landing))?;

        find_hole_far_exit_in_sector(&hole.points, entry, landing)
    }
}

/// Extend a trajectory through one already-resolved hole sector. This must
/// not search the global hole registry: a raised dry obstacle may overlap a
/// ground-level hole in projection, and Original extends through only the
/// `RHSectorMaterial*` returned for the actual impact obstacle.
pub fn find_hole_far_exit_in_sector(
    sector_points: &[MapPoint],
    entry: MapPoint,
    landing: MapPoint,
) -> Option<MapPoint> {
    let mut best = None;
    let mut best_y = landing.y + 2000.0;
    let n = sector_points.len();
    if n < 3 {
        return None;
    }
    for i in 0..n {
        let a = sector_points[i];
        let b = sector_points[(i + 1) % n];
        let Some(isec) = segment_line_intersection(entry, landing, a, b) else {
            continue;
        };
        if isec.y > landing.y && isec.y < best_y {
            best_y = isec.y;
            best = Some(isec);
        }
    }
    best
}

/// Resolve water/hole material in the same scope as Original's
/// `DetermineWaterHole`: material sub-sectors of the exact impact obstacle
/// when one exists, otherwise the global ground sector registry.
pub fn determine_water_hole_scoped<'a>(
    water_zones: &'a WaterZones,
    obstacle: Option<&'a crate::sight_obstacle::SightObstacle>,
    point: MapPoint,
) -> Option<WaterHoleResolution<'a>> {
    use crate::element::GameMaterial;

    let Some(obstacle) = obstacle else {
        return water_zones.zones.iter().find_map(|zone| {
            zone.contains(point).then_some(WaterHoleResolution {
                material: zone.material,
                sector_points: matches!(zone.material, Material::Hole)
                    .then_some(zone.points.as_slice()),
            })
        });
    };

    let obstacle_material = GameMaterial::from_u32(obstacle.material as u32);
    assert!(
        !matches!(obstacle_material, GameMaterial::Hole),
        "overall sight-obstacle material must not be HOLE"
    );
    if matches!(obstacle_material, GameMaterial::Water) {
        if obstacle
            .material_sectors
            .iter()
            .any(|sector| sector.contains(point))
        {
            return None;
        }
        return Some(WaterHoleResolution {
            material: Material::Water,
            sector_points: None,
        });
    }

    obstacle.material_sectors.iter().find_map(|sector| {
        if !sector.contains(point) {
            return None;
        }
        match sector.material {
            GameMaterial::Water => Some(WaterHoleResolution {
                material: Material::Water,
                sector_points: None,
            }),
            GameMaterial::Hole => Some(WaterHoleResolution {
                material: Material::Hole,
                sector_points: Some(sector.points.as_slice()),
            }),
            _ => None,
        }
    })
}

/// Obstacle-anchored variant of [`WaterZones::determine_water_hole`].
///
/// **Branch 2 — water-material obstacle (e.g. a lake modelled as a
/// sight-obstacle with material WATER):** if `point` lies inside any
/// of the obstacle's material sub-sectors, returns `None` (the impact
/// is on a dry island within the lake — no splash). Otherwise returns
/// `Some(Water)` (the obstacle as a whole is water → splash).
///
/// **Branch 3 — non-water obstacle (e.g. a roof with a hole punched
/// out, a stone floor with a puddle):** scans sub-sectors and returns
/// the first WATER/HOLE sub-sector whose polygon contains `point`. If
/// none match, returns `None`. We don't assert the obstacle's overall
/// material is never HOLE because asset data may legitimately drift —
/// a HOLE-material obstacle would simply pick this branch and look
/// for sub-sector overrides like any other non-water obstacle.
pub fn determine_water_hole_with_obstacle(
    obstacle: &crate::sight_obstacle::SightObstacle,
    point: MapPoint,
) -> Option<Material> {
    let empty_ground_zones = WaterZones::new();
    determine_water_hole_scoped(&empty_ground_zones, Some(obstacle), point)
        .map(|resolution| resolution.material)
}

/// Intersect an infinite line through `line_a→line_b` with the finite
/// segment `seg_a→seg_b`.  Returns the intersection point in 2D if it
/// lies strictly inside the segment, else `None`.  Parallel / colinear
/// lines are treated as no intersection.
fn segment_line_intersection(
    line_a: MapPoint,
    line_b: MapPoint,
    seg_a: MapPoint,
    seg_b: MapPoint,
) -> Option<MapPoint> {
    let rx = line_b.x - line_a.x;
    let ry = line_b.y - line_a.y;
    let sx = seg_b.x - seg_a.x;
    let sy = seg_b.y - seg_a.y;
    let rxs = rx * sy - ry * sx;
    if rxs.abs() < 1e-6 {
        return None;
    }
    let qpx = seg_a.x - line_a.x;
    let qpy = seg_a.y - line_a.y;
    let u = (qpx * ry - qpy * rx) / rxs;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    Some(MapPoint::new(seg_a.x + u * sx, seg_a.y + u * sy))
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
    fn ignores_non_water_materials() {
        let raw = vec![square(0, 0, 10), square(3, 0, 10)];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(zones.is_empty());
    }

    #[test]
    fn detects_water_and_hole() {
        let raw = vec![square(5, 0, 10), square(8, 20, 30)];
        let zones = WaterZones::build_from_raw(&raw);
        assert_eq!(zones.zones.len(), 2);
        assert_eq!(
            zones.determine_water_hole(MapPoint::new(5.0, 5.0)),
            Some(Material::Water)
        );
        assert_eq!(
            zones.determine_water_hole(MapPoint::new(25.0, 25.0)),
            Some(Material::Hole)
        );
        assert_eq!(zones.determine_water_hole(MapPoint::new(15.0, 15.0)), None);
    }

    #[test]
    fn rejects_degenerate_polygons() {
        let raw = vec![RawMaterialSector {
            material: 5,
            polygon: SectorPolygon {
                points: vec![(0, 0), (1, 1)],
            },
        }];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(zones.is_empty());
    }

    /// Far-edge exit on a square hole picks the polygon edge with the
    /// smallest screen-Y that is strictly greater than the landing's Y
    /// — the isometric +Y-direction filter.  A projectile entering at
    /// (5,3) and landing at (5,5) moving in +y should exit at y=10 on
    /// the bottom edge of the square.
    #[test]
    fn find_hole_far_exit_finds_far_polygon_edge() {
        let raw = vec![square(8, 0, 10)];
        let zones = WaterZones::build_from_raw(&raw);
        let exit = zones.find_hole_far_exit(MapPoint::new(5.0, 3.0), MapPoint::new(5.0, 5.0));
        let exit = exit.expect("trajectory extending through hole should exit the far edge");
        assert!((exit.x - 5.0).abs() < 1e-3);
        assert!(
            (exit.y - 10.0).abs() < 1e-3,
            "y should be at far edge, got {}",
            exit.y
        );
    }

    /// A landing outside any hole returns None — the caller should not
    /// extend the trajectory.
    #[test]
    fn find_hole_far_exit_none_for_non_hole_landings() {
        let raw = vec![square(8, 0, 10)];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(
            zones
                .find_hole_far_exit(MapPoint::new(20.0, 20.0), MapPoint::new(20.0, 25.0))
                .is_none()
        );
    }

    /// Water (not hole) is ignored — the fall-into-hole trajectory is
    /// only invoked for the HOLE material branch.
    #[test]
    fn find_hole_far_exit_ignores_water_zones() {
        let raw = vec![square(5, 0, 10)];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(
            zones
                .find_hole_far_exit(MapPoint::new(5.0, 3.0), MapPoint::new(5.0, 5.0))
                .is_none()
        );
    }

    /// A purely-horizontal flight across a hole has no forward-Y
    /// edge (both x-edges of the square sit at the same screen-Y as
    /// the landing), so this returns None.  This exercises a previous
    /// divergence where Rust picked a trajectory-aligned edge that
    /// the screen-Y selection rule would never select.
    #[test]
    fn find_hole_far_exit_rejects_horizontal_flight() {
        let raw = vec![square(8, 0, 10)];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(
            zones
                .find_hole_far_exit(MapPoint::new(3.0, 5.0), MapPoint::new(5.0, 5.0))
                .is_none()
        );
    }

    fn make_obstacle(
        material_code: u8,
        sub_sectors: Vec<crate::material_sectors::MaterialSector>,
    ) -> crate::sight_obstacle::SightObstacle {
        let mut obs = crate::sight_obstacle::SightObstacle::new(0, 0);
        obs.material = material_code;
        obs.material_sectors = sub_sectors;
        obs
    }

    fn material_sector(
        material: crate::element::GameMaterial,
        min: f32,
        max: f32,
    ) -> crate::material_sectors::MaterialSector {
        let points = vec![
            MapPoint::new(min, min),
            MapPoint::new(max, min),
            MapPoint::new(max, max),
            MapPoint::new(min, max),
        ];
        let mut bbox = MapBBox::new();
        for &p in &points {
            bbox.expand_point(p);
        }
        crate::material_sectors::MaterialSector {
            points,
            bounding_box: bbox,
            material,
        }
    }

    /// Branch 2 — water-material obstacle, no sub-sector hit. Returns
    /// `Some(Water)` → projectile splashes.
    #[test]
    fn water_obstacle_splashes_when_no_sub_sector_hit() {
        let obs = make_obstacle(5 /* WATER */, vec![]);
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(5.0, 5.0)),
            Some(Material::Water)
        );
    }

    /// Branch 2 — water-material obstacle with a sub-sector covering
    /// the impact. Models a "land island" within a lake — the no-splash
    /// sentinel, here `None`.  Sub-sector material is irrelevant.
    #[test]
    fn water_obstacle_dry_sub_sector_gives_no_splash() {
        let obs = make_obstacle(
            5,
            vec![material_sector(
                crate::element::GameMaterial::Stone,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(5.0, 5.0)),
            None
        );
    }

    /// Branch 2 — sub-sector exists but impact is outside it →
    /// fallthrough to splash on the lake.
    #[test]
    fn water_obstacle_off_sub_sector_still_splashes() {
        let obs = make_obstacle(
            5,
            vec![material_sector(
                crate::element::GameMaterial::Stone,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(50.0, 50.0)),
            Some(Material::Water)
        );
    }

    /// Branch 3 — non-water obstacle (e.g. a stone roof) with a HOLE
    /// sub-sector punched out. Impact inside the hole returns
    /// `Some(Hole)`.
    #[test]
    fn non_water_obstacle_with_hole_sub_sector_returns_hole() {
        let obs = make_obstacle(
            2, /* STONE */
            vec![material_sector(
                crate::element::GameMaterial::Hole,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(5.0, 5.0)),
            Some(Material::Hole)
        );
    }

    /// Branch 3 — non-water obstacle with a WATER sub-sector
    /// (a puddle). Impact inside returns `Some(Water)`.
    #[test]
    fn non_water_obstacle_with_water_sub_sector_returns_water() {
        let obs = make_obstacle(
            1, /* WOOD */
            vec![material_sector(
                crate::element::GameMaterial::Water,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(5.0, 5.0)),
            Some(Material::Water)
        );
    }

    /// Branch 3 — non-water obstacle with no water/hole sub-sector
    /// (e.g. a stone roof with only a wood-floor inset) returns
    /// `None`. The polygon test is gated on `material == Water ||
    /// material == Hole`, so non-water/hole sub-sectors are skipped
    /// without affecting the outcome.
    #[test]
    fn non_water_obstacle_ignores_non_water_sub_sectors() {
        let obs = make_obstacle(
            2,
            vec![material_sector(
                crate::element::GameMaterial::Wood,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(5.0, 5.0)),
            None
        );
    }

    /// Branch 3 — non-water obstacle, water/hole sub-sector exists
    /// but impact is outside → no splash.
    #[test]
    fn non_water_obstacle_off_sub_sector_returns_none() {
        let obs = make_obstacle(
            2,
            vec![material_sector(
                crate::element::GameMaterial::Hole,
                0.0,
                10.0,
            )],
        );
        assert_eq!(
            determine_water_hole_with_obstacle(&obs, MapPoint::new(50.0, 50.0)),
            None
        );
    }

    #[test]
    fn landing_is_in_hole_detects_hole_polygons() {
        let raw = vec![square(5, 0, 10), square(8, 20, 30)];
        let zones = WaterZones::build_from_raw(&raw);
        assert!(!zones.landing_is_in_hole(MapPoint::new(5.0, 5.0)));
        assert!(zones.landing_is_in_hole(MapPoint::new(25.0, 25.0)));
        assert!(!zones.landing_is_in_hole(MapPoint::new(15.0, 15.0)));
    }

    #[test]
    fn raised_dry_obstacle_does_not_inherit_overlapping_global_hole() {
        let zones = WaterZones::build_from_raw(&[square(8, 0, 10)]);
        let obstacle = make_obstacle(2 /* STONE */, vec![]);

        assert!(
            determine_water_hole_scoped(&zones, Some(&obstacle), MapPoint::new(5.0, 5.0)).is_none(),
            "an exact obstacle impact must not fall through to projected ground sectors"
        );
    }

    #[test]
    fn obstacle_hole_retains_its_exact_polygon_for_extension() {
        let zones = WaterZones::build_from_raw(&[square(8, -100, 100)]);
        let obstacle_hole = material_sector(crate::element::GameMaterial::Hole, 0.0, 10.0);
        let obstacle = make_obstacle(2 /* STONE */, vec![obstacle_hole.clone()]);

        let resolution =
            determine_water_hole_scoped(&zones, Some(&obstacle), MapPoint::new(5.0, 5.0))
                .expect("hole sub-sector should classify as a hole");
        assert_eq!(resolution.material, Material::Hole);
        assert_eq!(
            resolution.sector_points,
            Some(obstacle_hole.points.as_slice())
        );
        assert_eq!(
            find_hole_far_exit_in_sector(
                resolution.sector_points.unwrap(),
                MapPoint::new(5.0, 3.0),
                MapPoint::new(5.0, 5.0),
            ),
            Some(MapPoint::new(5.0, 10.0)),
            "extension must use the obstacle's small hole, not the overlapping global polygon"
        );
    }

    #[test]
    fn bare_ground_hole_uses_global_sector_and_retains_polygon() {
        let zones = WaterZones::build_from_raw(&[square(8, 0, 10)]);
        let resolution =
            determine_water_hole_scoped(&zones, None, MapPoint::new(5.0, 5.0)).unwrap();

        assert_eq!(resolution.material, Material::Hole);
        assert_eq!(
            resolution.sector_points,
            Some(zones.zones[0].points.as_slice())
        );
    }

    #[test]
    fn water_obstacle_dry_island_stays_dry_under_scoped_lookup() {
        let zones = WaterZones::build_from_raw(&[square(5, 0, 10)]);
        let obstacle = make_obstacle(
            5,
            vec![material_sector(
                crate::element::GameMaterial::Stone,
                0.0,
                10.0,
            )],
        );

        assert!(
            determine_water_hole_scoped(&zones, Some(&obstacle), MapPoint::new(5.0, 5.0)).is_none()
        );
    }

    #[test]
    #[should_panic(expected = "overall sight-obstacle material must not be HOLE")]
    fn overall_hole_obstacle_violates_original_invariant() {
        let zones = WaterZones::new();
        let obstacle = make_obstacle(8 /* HOLE */, vec![]);
        let _ = determine_water_hole_scoped(&zones, Some(&obstacle), MapPoint::new(0.0, 0.0));
    }
}
