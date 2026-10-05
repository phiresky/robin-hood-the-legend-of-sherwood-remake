//! Configuration-space routing with separate foot support and center ownership.

use super::*;
use geo::{
    BooleanOps, Buffer, Closest, ClosestPoint, ConvexHull, MultiPoint, MultiPolygon, Relate,
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
        let mut support = MultiPolygon::from(vec![floor.clone()]);
        for landing in landings {
            support = support.union(&convert(landing)?);
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
        for obstacle in &self.obstacles {
            let obstacle = convert(obstacle)?;
            centers = centers.difference(&obstacle);
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
        // A polygonal free space has a shortest path through visible boundary
        // vertices. Retain hole vertices too; they represent blocked footprints.
        let mut points = vec![source, goal];
        for ring in std::iter::once(region.exterior()).chain(region.interiors()) {
            points.extend(ring.points().take(ring.0.len().saturating_sub(1)));
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

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
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
