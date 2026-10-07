//! Ordinary floor queries in physical XY, independent of screen projection.

use super::*;
use geo::{Closest, ClosestPoint, ConvexHull, MultiPoint, MultiPolygon, Relate};
use robin_level_data::stair_navigation::StairNavigationPlane;

/// A current collision snapshot of one planar receiving floor. Callers rebuild
/// `obstacles` from live motion state and nearby actors before each query.
/// This is ordinary walking geometry, not a lift or a connection to another floor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalWalkingSurface {
    pub boundary: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
    pub plane: [f64; 3],
    pub obstacles: Vec<Vec<[f64; 2]>>,
}

impl PhysicalWalkingSurface {
    fn geometry(&self) -> Result<(Polygon<f64>, Vec<Polygon<f64>>), String> {
        StairNavigationPlane::new(self.plane)?;
        let boundary = polygon(&self.boundary)?;
        let holes = self
            .holes
            .iter()
            .map(|ring| polygon(ring))
            .collect::<Result<Vec<_>, _>>()?;
        let floor = Polygon::new(
            boundary.exterior().clone(),
            holes.iter().map(|p| p.exterior().clone()).collect(),
        );
        if !floor.is_valid() || floor.unsigned_area() <= 0.0 {
            return Err("physical walking floor has invalid holes".into());
        }
        let solids = self
            .obstacles
            .iter()
            .map(|ring| polygon(ring))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((floor, solids))
    }

    fn clearance(half: MoveBoxHalfDiagonal, inset: f64) -> Result<Vec<[f64; 2]>, String> {
        if !half.x.is_finite() || !half.y.is_finite() || half.x <= 1.0 || half.y <= 1.0 {
            return Err("physical walking query requires a finite positive footprint".into());
        }
        Ok(super::clearance::rectangle_offsets(half, inset))
    }

    /// Normal walking uses the movement inset. Endpoints must already occupy
    /// this receiving plane; a screen coordinate cannot select a floor here.
    pub fn route(
        &self,
        source: [f32; 3],
        goal: [f32; 3],
        half: MoveBoxHalfDiagonal,
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        let clearance = Self::clearance(half, 1.0)?;
        let (floor, solids) = self.geometry()?;
        let plane = StairNavigationPlane::new(self.plane)?;
        if source.iter().chain(&goal).any(|v| !v.is_finite()) {
            return Err("physical walking endpoints must be finite".into());
        }
        if !plane.contains_runtime_position(source) || !plane.contains_runtime_position(goal) {
            return Ok(None);
        }
        super::landing_support::route_on_surface(
            &floor,
            &solids,
            [source[0], source[1]],
            [goal[0], goal[1]],
            &clearance,
            &[],
        )
    }

    /// Recover an overlapping footprint while keeping the center on the same
    /// real floor. The full source box, plus the command's half-unit margin,
    /// must fit at the result. Never jump across a hole or a live solid, invent
    /// support outside the receiver, or accept an unbounded displacement.
    /// Sources already beyond the receiving floor require separate fall handling.
    pub fn recover_source(
        &self,
        source: [f32; 3],
        half: MoveBoxHalfDiagonal,
        max_distance: f64,
    ) -> Result<Option<[f64; 3]>, String> {
        let full = Self::clearance(half, 0.0)?;
        let recovery = Self::clearance(half, -0.5)?;
        let (floor, solids) = self.geometry()?;
        let plane = StairNavigationPlane::new(self.plane)?;
        if source.iter().any(|v| !v.is_finite()) || !max_distance.is_finite() || max_distance < 0.0
        {
            return Err(
                "physical source recovery requires finite coordinates and a nonnegative limit"
                    .into(),
            );
        }
        if !plane.contains_runtime_position(source) {
            return Ok(None);
        }
        let source = Point::new(f64::from(source[0]), f64::from(source[1]));
        if !floor.relate(&source).is_covers()
            || solids.iter().any(|solid| solid.intersects(&source))
        {
            return Ok(None);
        }
        let support = MultiPolygon::from(vec![floor.clone()]);
        let full_centers = super::landing_support::clearance_centers(&support, &solids, &full);
        if full_centers.relate(&source).is_covers() {
            return Ok(Some(plane.world_position([source.x(), source.y()])?));
        }
        let centers = super::landing_support::clearance_centers(&support, &solids, &recovery);
        let mut free = support;
        for solid in &solids {
            free = free.difference(solid);
        }
        let footprint = |point: Point<f64>| {
            MultiPoint::from_iter(
                recovery
                    .iter()
                    .map(|[x, y]| Point::new(point.x() + x, point.y() + y)),
            )
            .convex_hull()
        };
        // The initial overlap is the reason for recovery. Permit leaving that
        // overlap, but reject a sweep which introduces any new unsupported area.
        let initial = footprint(source);
        let allowed = free.union(&initial);
        let mut best: Option<(f64, [f64; 3])> = None;
        for region in centers {
            let Closest::SinglePoint(point) = region.closest_point(&source) else {
                continue;
            };
            let segment = geo::Line::new(source.0, point.0);
            if !floor.relate(&segment).is_covers()
                || solids.iter().any(|solid| solid.intersects(&segment))
            {
                continue;
            }
            let swept = MultiPoint::from_iter(
                initial
                    .exterior()
                    .points()
                    .chain(footprint(point).exterior().points()),
            )
            .convex_hull();
            // Use the same polygon difference as clearance construction. An
            // independent edge-relation test can disagree at a computed tangent
            // even when subtraction leaves no unsupported region.
            if !swept.difference(&allowed).0.is_empty() {
                continue;
            }
            let distance =
                plane.route_distance([source.x(), source.y()], [point.x(), point.y()])?;
            if distance <= max_distance
                && best.as_ref().is_none_or(|(current, _)| distance < *current)
            {
                best = Some((distance, plane.world_position([point.x(), point.y()])?));
            }
        }
        Ok(best.map(|(_, point)| point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(plane: [f64; 3]) -> PhysicalWalkingSurface {
        PhysicalWalkingSurface {
            boundary: vec![[0., 0.], [60., 0.], [60., 40.], [0., 40.]],
            holes: vec![],
            plane,
            obstacles: vec![],
        }
    }

    fn world(floor: &PhysicalWalkingSurface, point: [f64; 2]) -> [f32; 3] {
        StairNavigationPlane::new(floor.plane)
            .unwrap()
            .world_position(point)
            .unwrap()
            .map(|v| v as f32)
    }

    #[test]
    fn compressed_and_edge_on_receivers_keep_physical_walk_routes() {
        for b in [0., 0.9216188788414001, 1., 1.2] {
            let floor = surface([0.6336818337440491, b, 100.]);
            let from = world(&floor, [10., 10.]);
            let to = world(&floor, [50., 30.]);
            assert_eq!(
                floor
                    .route(from, to, MoveBoxHalfDiagonal::new(6., 3.))
                    .unwrap(),
                Some(vec![[10., 10.], [50., 30.]])
            );
            let mut wrong_height = from;
            wrong_height[2] += 10.;
            assert!(
                floor
                    .route(wrong_height, to, MoveBoxHalfDiagonal::new(6., 3.))
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn recovery_uses_full_box_and_keeps_the_receiving_height() {
        let floor = surface([0., 0.92, 100.]);
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let from = world(&floor, [5.8, 10.]);
        assert!(
            floor.route(from, from, half).unwrap().is_some(),
            "the normal inset fits"
        );
        assert!(floor.recover_source(from, half, 0.5).unwrap().is_none());
        let recovered = floor.recover_source(from, half, 2.).unwrap().unwrap();
        assert!((recovered[0] - 6.5).abs() < 1e-8);
        assert!((recovered[2] - 109.2).abs() < 1e-8);
        let supported = world(&floor, [6.1, 10.]);
        assert_eq!(
            floor.recover_source(supported, half, 0.).unwrap().unwrap()[0],
            f64::from(supported[0]),
            "do not apply the extra margin to an already authorized source"
        );
        assert!(
            floor
                .recover_source(world(&floor, [-1., 10.]), half, 100.)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn holes_and_current_solids_remain_authoritative() {
        let mut floor = surface([0., 0.9, 100.]);
        floor
            .holes
            .push(vec![[25., 10.], [35., 10.], [35., 30.], [25., 30.]]);
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let from = world(&floor, [10., 20.]);
        let to = world(&floor, [50., 20.]);
        let route = floor.route(from, to, half).unwrap().unwrap();
        assert!(route.len() > 2, "route must go around the hole");
        assert!(
            floor
                .recover_source(world(&floor, [30., 20.]), half, 100.)
                .unwrap()
                .is_none()
        );
        floor
            .obstacles
            .push(vec![[28., 0.], [32., 0.], [32., 40.], [28., 40.]]);
        assert!(floor.route(from, to, half).unwrap().is_none());
        floor.obstacles.clear();
        assert!(floor.route(from, to, half).unwrap().is_some());
    }

    #[test]
    fn narrow_receivers_do_not_acquire_fake_recovery_space() {
        let mut floor = surface([0., 1., 100.]);
        floor.boundary = vec![[0., 0.], [8., 0.], [8., 40.], [0., 40.]];
        let from = world(&floor, [4., 10.]);
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(floor.route(from, from, half).unwrap().is_none());
        assert!(floor.recover_source(from, half, 100.).unwrap().is_none());
    }

    #[test]
    fn compiled_rotated_roof_recovers_without_extrapolating_its_receiver() {
        #[derive(Serialize, Deserialize)]
        struct Fixture {
            surface: PhysicalWalkingSurface,
            source: [f32; 3],
        }
        let Fixture { surface, source } =
            serde_json::from_str(include_str!("fixtures/rotated-roof-approach.json")).unwrap();
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(surface.route(source, source, half).unwrap().is_some());
        assert!(
            surface.recover_source(source, half, 0.).unwrap().is_none(),
            "the full standing footprint does not fit at the authored outside point"
        );
        let recovered = surface
            .recover_source(source, half, 3.)
            .unwrap()
            .expect("nearby physical support must allow bounded recovery");
        let point = recovered.map(|v| v as f32);
        assert!(
            surface.route(point, source, half).unwrap().is_some(),
            "the recovered source must retain a supported approach to the doorway"
        );
        assert!((recovered[2] - f64::from(source[2])).abs() < 3.);
    }

    #[test]
    fn malformed_geometry_and_nonfinite_queries_are_errors() {
        let mut floor = surface([0., 0., 0.]);
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(
            floor
                .route([f32::NAN, 10., 0.], [20., 20., 0.], half)
                .is_err()
        );
        assert!(
            floor
                .recover_source([10., 10., 0.], half, f64::INFINITY)
                .is_err()
        );
        floor
            .holes
            .push(vec![[55., 10.], [65., 10.], [65., 20.], [55., 20.]]);
        assert!(floor.route([10., 10., 0.], [20., 20., 0.], half).is_err());
    }
}
