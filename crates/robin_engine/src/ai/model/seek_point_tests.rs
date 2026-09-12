use super::{Position, SeekPoint, SeekPointDirection};

fn direction(x: f32, y: f32, value: u16) -> SeekPointDirection {
    SeekPointDirection {
        position: Position {
            x,
            y,
            ..Position::default()
        },
        direction: value,
    }
}

#[test]
fn add_if_near_handles_duplicate_without_growing_unique_directions() {
    let first = direction(100.0, 200.0, 7);
    let mut point = SeekPoint::from_direction(&first);

    assert!(point.add_if_near(&direction(110.0, 190.0, 7)));
    assert_eq!(point.directions, vec![7]);
}

#[test]
fn add_if_near_appends_distinct_direction() {
    let first = direction(100.0, 200.0, 7);
    let mut point = SeekPoint::from_direction(&first);

    assert!(point.add_if_near(&direction(110.0, 190.0, 12)));
    assert_eq!(point.directions, vec![7, 12]);
}

#[test]
fn add_if_near_rejects_direction_outside_tolerance() {
    let first = direction(100.0, 200.0, 7);
    let mut point = SeekPoint::from_direction(&first);

    assert!(!point.add_if_near(&direction(111.0, 200.0, 12)));
    assert_eq!(point.directions, vec![7]);
}
