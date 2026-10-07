//! Configuration-space routing with separate foot support and center ownership.

use super::*;
use geo::algorithm::buffer::{BufferStyle, LineJoin};
use geo::{
    BooleanOps, BoundingRect, Buffer, Closest, ClosestPoint, ConvexHull, MultiPoint, MultiPolygon,
    Relate, Simplify,
};

impl StairRouteGeometry {
    /// Landings supply foot support only. The actor center remains on this
    /// stair, so a detour never extrapolates its height plane onto a landing.
    /// Callers must supply only connected, height-matched landing geometry and
    /// include its current collision in `obstacles`.
    pub fn route_with_landing_support(
        &self,
        source: [f32; 2],
        goal: [f32; 2],
        half: MoveBoxHalfDiagonal,
        landings: &[Vec<[f32; 2]>],
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        let landings = landings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| [f64::from(p[0]), f64::from(p[1])])
                    .collect()
            })
            .collect::<Vec<_>>();
        self.route_with_precise_landing_support(source, goal, half, &landings, &[])
    }

    /// Keep intersection vertices in their computed precision. Rounding a thin
    /// clipped solid back to f32 can turn valid edges into crossing spikes.
    pub(super) fn route_with_precise_landing_support(
        &self,
        source: [f32; 2],
        goal: [f32; 2],
        half: MoveBoxHalfDiagonal,
        landings: &[Vec<[f64; 2]>],
        precise_obstacles: &[Vec<[f64; 2]>],
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        let regions = landings
            .iter()
            .map(|ring| polygon(ring))
            .collect::<Result<Vec<_>, _>>()?;
        self.route_with_landing_regions(source, goal, half, &regions, precise_obstacles)
    }

    pub(super) fn route_with_landing_regions(
        &self,
        source: [f32; 2],
        goal: [f32; 2],
        half: MoveBoxHalfDiagonal,
        landings: &[Polygon<f64>],
        precise_obstacles: &[Vec<[f64; 2]>],
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        if source.iter().chain(&goal).any(|value| !value.is_finite())
            || !half.x.is_finite()
            || !half.y.is_finite()
            || half.x <= 1.0
            || half.y <= 1.0
        {
            return Err(
                "physical landing query requires finite points and a positive footprint".into(),
            );
        }
        let convert = |ring: &[[f32; 2]]| -> Result<Polygon<f64>, String> {
            polygon(ring)?;
            Ok(Polygon::new(
                LineString::from(
                    ring.iter()
                        .map(|p| (f64::from(p[0]), f64::from(p[1])))
                        .collect::<Vec<_>>(),
                ),
                Vec::new(),
            ))
        };
        let floor = convert(&self.boundary)?;
        // Centers stay on this stair. Only support within one footprint of its
        // bounds can affect them; distant terrain must not multiply erosion
        // and visibility work. The full half size leaves one unit beyond the
        // effective footprint used below, so clipping cannot create a boundary
        // that excludes otherwise supported stair centers.
        let bounds = floor.bounding_rect().ok_or("empty physical stair")?;
        let neighborhood = geo::Rect::new(
            (
                bounds.min().x - f64::from(half.x),
                bounds.min().y - f64::from(half.y),
            ),
            (
                bounds.max().x + f64::from(half.x),
                bounds.max().y + f64::from(half.y),
            ),
        )
        .to_polygon();
        let mut support = MultiPolygon::from(vec![floor.clone()]);
        for landing in landings {
            if !landing.is_valid() {
                return Err("physical landing support region is invalid".into());
            }
            support = support.union(&landing.intersection(&neighborhood));
        }
        // Independently encoded f32 edges can leave sub-ULP cracks between
        // already bound floors. Close only that representation error, then
        // remove the expansion before footprint erosion. Actor centers remain
        // constrained to the stair and live solids are subtracted below.
        if !landings.is_empty() {
            let rounding = self
                .boundary
                .iter()
                .flatten()
                .fold(1.0_f64, |scale, value| scale.max(f64::from(value.abs())))
                * f64::from(f32::EPSILON);
            // Bevels keep the correction bounded without creating circular
            // micro-segments that would multiply visibility-graph vertices.
            support = support
                .buffer_with_style(BufferStyle::new(rounding).line_join(LineJoin::Bevel))
                .buffer_with_style(BufferStyle::new(-rounding).line_join(LineJoin::Bevel));
        }
        let half = [f64::from(half.x - 1.0), f64::from(half.y - 1.0)];
        let sweep = |a: geo::Coord<f64>, b: geo::Coord<f64>| {
            MultiPoint::from_iter([a, b].into_iter().flat_map(|p| {
                [-half[0], half[0]].into_iter().flat_map(move |x| {
                    [-half[1], half[1]]
                        .into_iter()
                        .map(move |y| Point::new(p.x + x, p.y + y))
                })
            }))
            .convex_hull()
        };
        let solids = self
            .obstacles
            .iter()
            .map(|ring| convert(ring))
            .chain(precise_obstacles.iter().map(|ring| polygon(ring)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flat_map(|solid| solid.intersection(&neighborhood).0)
            .collect::<Vec<_>>();
        let direct = geo::Line::new(
            (f64::from(source[0]), f64::from(source[1])),
            (f64::from(goal[0]), f64::from(goal[1])),
        );
        let footprint = sweep(direct.start, direct.end);
        // A completely supported swept footprint proves a straight route
        // without constructing all possible actor centers. Boundary/rounding
        // cases still use the full configuration-space query below.
        if floor.relate(&direct).is_covers()
            && support.relate(&footprint).is_contains_properly()
            && solids.iter().all(|solid| !solid.intersects(&footprint))
        {
            return Ok(Some(vec![source, goal]));
        }
        // Erode real support by the effective rectangular footprint. Sweeping
        // every exterior and hole edge removes exactly the centers whose box
        // crosses a support boundary; no arbitrary padding is introduced.
        let mut centers = support.clone();
        for polygon in &support {
            for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
                for edge in ring.lines() {
                    centers = centers.difference(&sweep(edge.start, edge.end));
                }
            }
        }
        // Expand each live solid by the same footprint, including concave
        // solids: the solid plus its swept boundary is its rectangular dilation.
        for obstacle in &solids {
            centers = centers.difference(obstacle);
            for edge in obstacle.exterior().lines() {
                centers = centers.difference(&sweep(edge.start, edge.end));
            }
        }
        centers = centers.intersection(&floor);
        let original_source = source;
        let original_goal = goal;
        let source = Point::new(f64::from(source[0]), f64::from(source[1]));
        let goal = Point::new(f64::from(goal[0]), f64::from(goal[1]));
        // Independently rounded f32 seam points can lie a fraction of an ULP
        // outside their rounded edge. Normalize only that representational
        // error; this is not an actor-sized source repair or landing extension.
        let normalize = |region: &Polygon<f64>, point: Point<f64>| {
            if region.relate(&point).is_covers() {
                return Some(point);
            }
            match region.closest_point(&point) {
                Closest::SinglePoint(closest) => {
                    let tolerance = point.x().abs().max(point.y().abs()).max(1.0)
                        * f64::from(f32::EPSILON)
                        * 2.0;
                    ((closest.x() - point.x()).hypot(closest.y() - point.y()) <= tolerance)
                        .then_some(closest)
                }
                _ => None,
            }
        };
        let Some((region, source, goal)) = centers.iter().find_map(|region| {
            Some((region, normalize(region, source)?, normalize(region, goal)?))
        }) else {
            return Ok(None);
        };
        if source == goal {
            return Ok(Some(vec![original_source, original_goal]));
        }
        // A committed f32 step can round onto either side of a tangent. Even
        // the closest-point calculation can leave a sub-ULP residual. Use the
        // same coordinate-error budget for visibility as for endpoint seating;
        // otherwise re-planning at an obstacle tangent can strand the actor.
        let rounding = source
            .x()
            .abs()
            .max(source.y().abs())
            .max(goal.x().abs())
            .max(goal.y().abs())
            .max(1.0)
            * f64::from(f32::EPSILON)
            * 2.0;
        let visibility_region = region.buffer(rounding);
        // A supported direct segment is already the shortest route. In
        // particular, continuing along a stair should not rebuild visibility
        // links to every receiving-boundary vertex on each movement tick.
        if visibility_region
            .relate(&geo::Line::new(source.0, goal.0))
            .is_covers()
        {
            return Ok(Some(vec![original_source, original_goal]));
        }
        // A polygonal free space has a shortest path through visible boundary
        // vertices. Retain hole vertices too; they represent blocked footprints.
        let mut points = vec![source, goal];
        for ring in std::iter::once(region.exterior()).chain(region.interiors()) {
            // Drop redundant candidate vertices within the coordinate-error
            // budget. Visibility still uses the complete region, so this does
            // not simplify collision or authorize a segment through a solid.
            let candidates = ring.simplify(rounding * 0.25);
            points.extend(
                candidates
                    .points()
                    .take(candidates.0.len().saturating_sub(1)),
            );
        }
        let mut distance = vec![f64::INFINITY; points.len()];
        let mut previous = vec![None; points.len()];
        let mut settled = vec![false; points.len()];
        distance[0] = 0.0;
        loop {
            let Some(current) = (0..points.len())
                .filter(|&i| !settled[i] && distance[i].is_finite())
                .min_by(|&a, &b| distance[a].total_cmp(&distance[b]))
            else {
                return Ok(None);
            };
            if current == 1 {
                break;
            }
            settled[current] = true;
            for next in 0..points.len() {
                if settled[next] {
                    continue;
                }
                let length = (points[current].x() - points[next].x())
                    .hypot(points[current].y() - points[next].y());
                if distance[current] + length >= distance[next] {
                    continue;
                }
                let segment = geo::Line::new(points[current].0, points[next].0);
                if visibility_region.relate(&segment).is_covers() {
                    distance[next] = distance[current] + length;
                    previous[next] = Some(current);
                }
            }
        }
        let mut route = Vec::new();
        let mut current = 1;
        loop {
            route.push([points[current].x() as f32, points[current].y() as f32]);
            if current == 0 {
                break;
            }
            current = previous[current].expect("reachable physical route lost its predecessor");
        }
        route.reverse();
        route[0] = original_source;
        *route.last_mut().expect("physical route has endpoints") = original_goal;
        Ok(Some(route))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_ground_landing_supports_fractional_seam_entry() {
        let geometry = StairRouteGeometry {
            boundary: vec![
                [1629.7446717863465, 1847.2859488701574],
                [1632.0756613365384, 1843.3199892169418],
                [1635.0756613365384, 1837.7809892169416],
                [1638.0756613365384, 1833.240989216942],
                [1642.0756613365384, 1827.701989216942],
                [1645.0756613365384, 1822.1629892169417],
                [1648.0756613365384, 1816.623989216942],
                [1651.0756613365384, 1811.0839892169417],
                [1654.0756613365384, 1805.544989216942],
                [1657.0756613365384, 1800.0059892169418],
                [1660.0756613365384, 1794.4669892169418],
                [1663.0756613365384, 1788.9269892169418],
                [1669.075738352722, 1777.8486151450718],
                [1703.07571345476, 1784.8487360758522],
                [1700.0756613365384, 1790.3879892169418],
                [1697.0756613365384, 1795.9269892169418],
                [1694.0756613365384, 1801.466989216942],
                [1691.0756613365384, 1807.0059892169418],
                [1688.0756613365384, 1812.5449892169418],
                [1685.0756613365384, 1818.083989216942],
                [1682.0756613365384, 1823.623989216942],
                [1679.0756613365384, 1829.162989216942],
                [1676.0756613365384, 1834.7019892169417],
                [1673.0756613365384, 1839.2409892169417],
                [1670.0756613365384, 1843.7809892169419],
                [1667.0756613365384, 1849.3199892169418],
                [1664.2417760249575, 1854.3884217604252],
            ],
            obstacles: vec![],
        };
        let landings = vec![vec![
            [0., 1434.2092],
            [0., 0.],
            [4000., 0.],
            [4000., 2257.7534],
            [0., 1434.2092],
        ]];
        assert!(
            geometry
                .route_with_landing_support(
                    [1685.1724, 1781.1627],
                    [1683.5591, 1788.9983],
                    MoveBoxHalfDiagonal::new(6., 3.),
                    &landings,
                )
                .unwrap()
                .is_some()
        );
    }

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn distant_landing_geometry_does_not_change_local_stair_routes() {
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let mut geometry = StairRouteGeometry {
            boundary: rectangle(0., 0., 100., 30.),
            obstacles: vec![rectangle(45., 0., 55., 14.)],
        };
        let small = vec![rectangle(-10., -10., 110., 0.)];
        let large = vec![rectangle(-10000., -10000., 10000., 0.)];
        let expected = geometry
            .route_with_landing_support([10., 0.], [90., 0.], half, &small)
            .unwrap()
            .expect("local obstacle has a supported detour");
        for i in 0..100 {
            let x = 200. + i as f32 * 20.;
            geometry.obstacles.push(rectangle(x, -100., x + 10., -50.));
        }
        let actual = geometry
            .route_with_landing_support([10., 0.], [90., 0.], half, &large)
            .unwrap()
            .expect("distant collision cannot remove the detour");
        assert_eq!(actual, expected);
        geometry.obstacles.push(rectangle(45., 14., 55., 30.));
        assert!(
            geometry
                .route_with_landing_support([10., 0.], [90., 0.], half, &large)
                .unwrap()
                .is_none(),
            "nearby collision still blocks the stair"
        );
    }

    #[test]
    fn landing_support_does_not_allow_a_detour_off_the_stair() {
        let mut geometry = StairRouteGeometry {
            boundary: rectangle(0., 48., 100., 52.),
            obstacles: vec![],
        };
        let landings = vec![rectangle(0., 20., 100., 48.), rectangle(0., 52., 100., 80.)];
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(
            geometry
                .route_with_landing_support([30., 50.], [70., 50.], half, &landings)
                .unwrap()
                .is_some()
        );
        let gapped = vec![
            rectangle(0., 20., 100., 47.9),
            rectangle(0., 52.1, 100., 80.),
        ];
        assert!(
            geometry
                .route_with_landing_support([30., 50.], [70., 50.], half, &gapped)
                .unwrap()
                .is_none()
        );
        geometry.obstacles.push(rectangle(48., 48., 52., 52.));
        // The surrounding landings provide ample space for a geometric detour,
        // but that detour must not be executed using the stair's height plane.
        assert!(
            geometry
                .route_with_landing_support([30., 50.], [70., 50.], half, &landings)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn seam_endpoints_work_at_rotations_without_bridging_missing_support() {
        for degrees in [0_f32, 37., 90., 180., 270.] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let transform = |p: [f32; 2]| {
                [
                    p[0] * cos - p[1] * sin + 300.,
                    p[0] * sin + p[1] * cos + 300.,
                ]
            };
            // Transform one shared set of endpoints so that adjoining polygons
            // retain exactly shared edges after floating-point placement.
            let shape = |y0, y1| {
                rectangle(0., y0, 100., y1)
                    .into_iter()
                    .map(transform)
                    .collect::<Vec<_>>()
            };
            let geometry = StairRouteGeometry {
                boundary: shape(48., 52.),
                obstacles: vec![],
            };
            let landings = vec![shape(20., 48.), shape(52., 80.)];
            let half = MoveBoxHalfDiagonal::new(6., 3.);
            for (source, goal) in [([50., 48.], [50., 52.]), ([50., 52.], [50., 48.])] {
                let source = transform(source);
                let goal = transform(goal);
                let route = geometry
                    .route_with_landing_support(source, goal, half, &landings)
                    .unwrap();
                assert!(
                    route.is_some(),
                    "rotation {degrees}: {source:?} -> {goal:?}"
                );
                assert_eq!(route.as_ref().unwrap().first(), Some(&source));
                assert_eq!(route.as_ref().unwrap().last(), Some(&goal));
                assert!(
                    geometry
                        .route_with_landing_support(source, goal, half, &[])
                        .unwrap()
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn live_solids_keep_holes_in_the_route() {
        let geometry = StairRouteGeometry {
            boundary: rectangle(0., 0., 100., 100.),
            obstacles: vec![rectangle(45., 30., 55., 70.)],
        };
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let route = geometry
            .route_with_landing_support([20., 50.], [80., 50.], half, &[])
            .unwrap()
            .unwrap();
        assert!(route.len() > 2);
        for segment in route.windows(2) {
            for step in 0..=100 {
                let t = step as f32 / 100.;
                let [x, y] = [
                    segment[0][0] * (1. - t) + segment[1][0] * t,
                    segment[0][1] * (1. - t) + segment[1][1] * t,
                ];
                assert!((5. - 0.001..=95.001).contains(&x));
                assert!((2. - 0.001..=98.001).contains(&y));
                assert!(
                    x <= 40.001 || x >= 59.999 || y <= 28.001 || y >= 71.999,
                    "footprint crosses solid at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn rounded_tangent_can_be_replanned_without_stranding_the_actor() {
        let radius = 4.0_f32 / (std::f32::consts::PI / 16.0).cos();
        let obstacle = (0..16)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 16.0;
                [400.0 + radius * angle.cos(), 350.0 + radius * angle.sin()]
            })
            .collect();
        let geometry = StairRouteGeometry {
            boundary: rectangle(380., 300., 420., 400.),
            obstacles: vec![obstacle],
        };
        let landings = vec![
            rectangle(380., 270., 420., 300.),
            rectangle(380., 400., 420., 430.),
        ];
        let route = geometry
            .route_with_landing_support(
                [390.95352, 347.83972],
                [400., 380.],
                MoveBoxHalfDiagonal::new(6., 3.),
                &landings,
            )
            .unwrap();
        assert!(
            route.is_some(),
            "a rounded tangent must retain a route around its neighbour"
        );
    }
}
