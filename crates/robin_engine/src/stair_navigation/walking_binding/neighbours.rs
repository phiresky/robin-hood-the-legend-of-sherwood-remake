//! Foot support across actual, height-matched receiving-floor seams.

use super::*;
use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::{BoundingRect, Buffer, MultiLineString, MultiPolygon};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WalkingNeighbour {
    floor: usize,
    support: Vec<Polygon<f64>>,
}

fn footprint(surface: &PhysicalWalkingSurface) -> Polygon<f64> {
    Polygon::new(
        polygon(&surface.boundary)
            .expect("bound walking boundary")
            .into_inner()
            .0,
        surface
            .holes
            .iter()
            .map(|ring| polygon(ring).expect("bound walking hole").into_inner().0)
            .collect(),
    )
}

fn edges(polygon: &Polygon<f64>) -> impl Iterator<Item = geo::Line<f64>> + '_ {
    std::iter::once(polygon.exterior())
        .chain(polygon.interiors())
        .flat_map(LineString::lines)
}

fn compatible_support(
    source: &Polygon<f64>,
    source_plane: [f64; 3],
    neighbour: &Polygon<f64>,
    neighbour_plane: [f64; 3],
) -> Vec<Polygon<f64>> {
    let tolerance = source
        .exterior()
        .0
        .iter()
        .fold(1.0_f64, |scale, p| scale.max(p.x.abs()).max(p.y.abs()))
        * f64::from(f32::EPSILON)
        * 2.;
    let bounds = source.bounding_rect().expect("bound walking footprint");
    let other = neighbour.bounding_rect().expect("bound walking footprint");
    if bounds.max().x + tolerance < other.min().x
        || other.max().x + tolerance < bounds.min().x
        || bounds.max().y + tolerance < other.min().y
        || other.max().y + tolerance < bounds.min().y
    {
        return vec![];
    }
    let delta = std::array::from_fn::<_, 3, _>(|i| source_plane[i] - neighbour_plane[i]);
    let half_ulp = |value: f64| {
        let rounded = value as f32;
        ((f64::from(rounded.next_up()) - f64::from(rounded))
            .max(f64::from(rounded) - f64::from(rounded.next_down())))
            * 0.5
    };
    let matches = |p: geo::Coord<f64>| {
        let error = (delta[0].abs() * half_ulp(p.x) + delta[1].abs() * half_ulp(p.y)).max(0.001);
        (delta[0] * p.x + delta[1] * p.y + delta[2]).abs() <= error
    };
    let mut result = vec![];
    // Underlying terrain must not replace the receiver beneath the character's
    // center, and overlapping elevated edges must not grant support to it.
    for patch in neighbour.difference(source) {
        let mut compatible = false;
        let mut incompatible = vec![];
        for first in edges(source) {
            for second in edges(&patch) {
                let Some(contact) = super::super::landing_binding::rounded_shared_edge_precise(
                    first, second, tolerance,
                ) else {
                    continue;
                };
                if matches(contact.start) && matches(contact.end) {
                    compatible = true;
                } else {
                    incompatible.push(LineString::from(vec![contact.start, contact.end]));
                }
            }
        }
        if !compatible {
            continue;
        }
        if incompatible.is_empty() {
            result.push(patch);
        } else {
            // Wider than query-time roundoff closure: the valid low edge of a
            // step cannot also authorize its elevated sides through terrain.
            result.extend(
                patch.difference(
                    &MultiLineString(incompatible).buffer_with_style(
                        BufferStyle::new(tolerance.max(1e-6) * 4.)
                            .line_cap(LineCap::Butt)
                            .line_join(LineJoin::Bevel),
                    ),
                ),
            );
        }
    }
    result
}

impl BoundPhysicalWalkingSurface {
    /// Precompute static connections once after the final receiver list is built.
    /// This supplies feet, not permission to move the center onto another floor.
    pub fn bind_neighbours(floors: &mut [Self]) {
        let footprints = floors
            .iter()
            .map(|floor| footprint(&floor.geometry))
            .collect::<Vec<_>>();
        let neighbours = floors
            .iter()
            .enumerate()
            .map(|(index, floor)| {
                floors
                    .iter()
                    .enumerate()
                    .filter_map(|(other_index, other)| {
                        if index == other_index
                            || floor.layer != other.layer
                            || floor.sector != other.sector
                        {
                            return None;
                        }
                        let support = compatible_support(
                            &footprints[index],
                            floor.geometry.plane,
                            &footprints[other_index],
                            other.geometry.plane,
                        );
                        (!support.is_empty()).then_some(WalkingNeighbour {
                            floor: other_index,
                            support,
                        })
                    })
                    .collect()
            })
            .collect::<Vec<_>>();
        for (floor, neighbours) in floors.iter_mut().zip(neighbours) {
            floor.neighbours = neighbours;
        }
    }

    /// Controls on the supporting receiver still block the portion of the
    /// footprint resting there, including after state restoration.
    pub fn neighbour_support(&self, floors: &[Self], pathfinder: &PathFinder) -> Vec<Polygon<f64>> {
        self.neighbours
            .iter()
            .flat_map(|neighbour| {
                let mut support = MultiPolygon::new(neighbour.support.clone());
                for obstacle in floors[neighbour.floor].snapshot(pathfinder).obstacles {
                    support =
                        support.difference(&polygon(&obstacle).expect("bound walking obstacle"));
                }
                support.0
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon<f64> {
        polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]).unwrap()
    }

    fn bound(shape: Polygon<f64>, area: usize) -> BoundPhysicalWalkingSurface {
        let ring = |line: &LineString<f64>| line.points().map(|p| [p.x(), p.y()]).collect();
        BoundPhysicalWalkingSurface {
            layer: 0,
            sector: 0,
            area,
            receivers: vec![area as u32],
            geometry: PhysicalWalkingSurface {
                boundary: ring(shape.exterior()),
                holes: shape.interiors().iter().map(ring).collect(),
                plane: [0.; 3],
                obstacles: vec![],
                support: vec![],
            },
            obstacle_states: vec![],
            neighbours: vec![],
        }
    }

    #[test]
    fn neighbour_controls_and_restoration_preserve_footprint_collision() {
        let mut floors = vec![
            bound(rectangle(0., 0., 20., 20.), 0),
            bound(rectangle(20., 0., 40., 20.), 1),
        ];
        floors[1]
            .geometry
            .obstacles
            .push(vec![[20., 0.], [24., 0.], [24., 20.], [20., 20.]]);
        floors[1].obstacle_states.push(2);
        BoundPhysicalWalkingSurface::bind_neighbours(&mut floors);
        let floors: Vec<BoundPhysicalWalkingSurface> =
            serde_json::from_str(&serde_json::to_string(&floors).unwrap()).unwrap();
        let mut pathfinder = PathFinder::new();
        pathfinder.states = vec![vec![0, 0]];
        let can_stand = |pathfinder: &PathFinder| {
            let mut surface = floors[0].snapshot(pathfinder);
            surface.support = floors[0].neighbour_support(&floors, pathfinder);
            surface
                .route(
                    [18., 10., 0.],
                    [18., 10., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.),
                )
                .unwrap()
                .is_some()
        };
        assert!(can_stand(&pathfinder));
        pathfinder.states[0][1] = 2;
        assert!(!can_stand(&pathfinder));
        pathfinder.states[0][1] = 0;
        assert!(can_stand(&pathfinder));
    }

    #[test]
    fn neighbour_layers_sectors_and_holes_do_not_supply_false_support() {
        for (layer, sector) in [(1, 0), (0, 1)] {
            let mut floors = vec![
                bound(rectangle(0., 0., 20., 20.), 0),
                bound(rectangle(20., 0., 40., 20.), 1),
            ];
            floors[1].layer = layer;
            floors[1].sector = sector;
            BoundPhysicalWalkingSurface::bind_neighbours(&mut floors);
            assert!(floors.iter().all(|floor| floor.neighbours.is_empty()));
        }
        let source = rectangle(0., 0., 20., 20.);
        let other = Polygon::new(
            rectangle(20., 0., 40., 20.).exterior().clone(),
            vec![rectangle(21., 5., 30., 15.).exterior().clone()],
        );
        let mut surface = bound(source, 0).geometry;
        surface.support = compatible_support(&footprint(&surface), [0.; 3], &other, [0.; 3]);
        assert!(!surface.support.is_empty());
        assert!(
            surface
                .route(
                    [18., 10., 0.],
                    [18., 10., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.)
                )
                .unwrap()
                .is_none()
        );
        // Foot support never transfers ownership of the character's center.
        surface.support = compatible_support(
            &footprint(&surface),
            [0.; 3],
            &rectangle(20., 0., 40., 20.),
            [0.; 3],
        );
        assert!(
            surface
                .route(
                    [18., 10., 0.],
                    [22., 10., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.)
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn floor_seams_require_edges_and_matching_heights() {
        for offset in [0., 1400., 24000.] {
            let source = rectangle(offset, offset, offset + 20., offset + 20.);
            let neighbour = rectangle(offset + 20., offset, offset + 40., offset + 20.);
            assert!(!compatible_support(&source, [0.; 3], &neighbour, [0.; 3]).is_empty());
            assert!(compatible_support(&source, [0.; 3], &neighbour, [0., 0., 0.1]).is_empty());
            let gap = rectangle(offset + 20.1, offset, offset + 40., offset + 20.);
            assert!(compatible_support(&source, [0.; 3], &gap, [0.; 3]).is_empty());
            let corner = rectangle(offset + 20., offset + 20., offset + 40., offset + 40.);
            assert!(compatible_support(&source, [0.; 3], &corner, [0.; 3]).is_empty());
        }
    }

    #[test]
    fn terrain_supports_low_step_edge_but_not_raised_sides() {
        let step = rectangle(0., 0., 20., 20.);
        let terrain = rectangle(-20., -20., 40., 40.);
        let support = compatible_support(&step, [0., 0.5, 0.], &terrain, [0.; 3]);
        assert!(!support.is_empty());
        let surface = PhysicalWalkingSurface {
            boundary: step.exterior().points().map(|p| [p.x(), p.y()]).collect(),
            holes: vec![],
            plane: [0., 0.5, 0.],
            obstacles: vec![],
            support,
        };
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(
            surface
                .route([10., 1., 0.5], [10., 5., 2.5], half)
                .unwrap()
                .is_some()
        );
        assert!(
            surface
                .route([1., 10., 5.], [5., 10., 5.], half)
                .unwrap()
                .is_none()
        );
        assert!(
            surface
                .route([10., 19., 9.5], [10., 15., 7.5], half)
                .unwrap()
                .is_none()
        );
    }
}
