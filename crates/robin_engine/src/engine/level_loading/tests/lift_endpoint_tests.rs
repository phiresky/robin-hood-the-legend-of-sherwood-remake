use super::lift_endpoint_door_indices;
use crate::coordinates::MapPoint;
use crate::gate::Door;
use crate::sector::SectorNumber;

fn lift_door(lift_sector: SectorNumber, point_in: MapPoint, point_out: MapPoint) -> Door {
    Door {
        sector_in: lift_sector,
        owning_lift_sector: Some(lift_sector),
        point_in,
        point_out,
        ..Door::default()
    }
}

#[test]
fn lift_endpoint_identity_uses_point_out_but_caches_point_in() {
    let lift = SectorNumber::new(42);
    let doors = vec![
        // High by the outside endpoint, despite having the lower
        // lift-side entry point in screen coordinates.
        lift_door(lift, MapPoint::new(10.0, 200.0), MapPoint::new(11.0, 10.0)),
        // Low by the outside endpoint, despite having the higher
        // lift-side entry point in screen coordinates.
        lift_door(lift, MapPoint::new(20.0, 0.0), MapPoint::new(21.0, 100.0)),
    ];

    let (low, high) = lift_endpoint_door_indices(&doors, lift).unwrap();

    assert_eq!((low, high), (1, 0));
    assert_eq!(doors[low as usize].point_in, MapPoint::new(20.0, 0.0));
    assert_eq!(doors[high as usize].point_in, MapPoint::new(10.0, 200.0));
}

#[test]
fn lift_endpoint_identity_excludes_reverse_edges_and_preserves_ties() {
    let lift = SectorNumber::new(42);
    let outside = SectorNumber::new(7);
    let mut reverse_edge = lift_door(
        outside,
        MapPoint::new(99.0, 99.0),
        MapPoint::new(99.0, 1000.0),
    );
    reverse_edge.sector_out = lift;
    let doors = vec![
        lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 10.0)),
        lift_door(lift, MapPoint::new(20.0, 100.0), MapPoint::new(20.0, 100.0)),
        reverse_edge,
        // Original uses strict comparisons, so an equal low endpoint
        // retains the first authored door.
        lift_door(lift, MapPoint::new(30.0, 100.0), MapPoint::new(30.0, 100.0)),
    ];

    assert_eq!(lift_endpoint_door_indices(&doors, lift), Some((1, 0)));
}

#[test]
fn lift_endpoint_identity_excludes_foreign_door_entering_same_sector() {
    let lift = SectorNumber::new(42);
    let mut foreign = Door {
        sector_in: lift,
        point_out: MapPoint::new(30.0, 101.0),
        ..Door::default()
    };
    foreign.owning_lift_sector = None;
    let doors = vec![
        foreign,
        lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 10.0)),
        lift_door(lift, MapPoint::new(20.0, 100.0), MapPoint::new(20.0, 100.0)),
    ];

    assert_eq!(lift_endpoint_door_indices(&doors, lift), Some((2, 1)));
}

#[test]
#[should_panic(expected = "fewer than two distinct high/low endpoint doors")]
fn lift_endpoint_identity_rejects_one_owned_door() {
    let lift = SectorNumber::new(42);
    let doors = vec![lift_door(
        lift,
        MapPoint::new(10.0, 10.0),
        MapPoint::new(10.0, 10.0),
    )];

    let _ = lift_endpoint_door_indices(&doors, lift);
}

#[test]
#[should_panic(expected = "fewer than two distinct high/low endpoint doors")]
fn lift_endpoint_identity_rejects_equal_outside_heights() {
    let lift = SectorNumber::new(42);
    let doors = vec![
        lift_door(lift, MapPoint::new(10.0, 10.0), MapPoint::new(10.0, 50.0)),
        lift_door(lift, MapPoint::new(20.0, 20.0), MapPoint::new(20.0, 50.0)),
    ];

    let _ = lift_endpoint_door_indices(&doors, lift);
}
