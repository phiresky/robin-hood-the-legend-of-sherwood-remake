use super::{drunken_deviation_direction, insert_drunken_orders_with};

#[test]
fn deviation_direction_uses_original_isometric_aspect_ratio() {
    let direction = 2;
    let (raw_x, raw_y) = crate::element_kinds::direction_vector_16(direction);
    let [x, y] = drunken_deviation_direction(direction);

    assert_eq!(x, raw_x);
    assert_eq!(y, raw_y * crate::position_interface::ASPECT_RATIO);
    assert_ne!(y, raw_y, "the bare compass vector overextends map-space Y");
}

#[test]
fn drunken_midpoint_follows_startup_transition_without_reheading_it() {
    let mut element = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::MoveOk,
        None,
        crate::order::OrderType::WalkingUpright,
    );
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
        0.0,
        -4.0,
        std::num::NonZeroU32::new(10).unwrap(),
    ));
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::WalkingUpright,
        0.0,
        -40.0,
        std::num::NonZeroU32::new(11).unwrap(),
    ));
    let mut next_order_id = 20;

    insert_drunken_orders_with(
        &mut element,
        crate::coordinates::MapPoint::ZERO,
        1,
        &mut next_order_id,
        |first, second| {
            Some(crate::coordinates::MapPoint::new(
                (first.x + second.x) * 0.5 + 3.0,
                (first.y + second.y) * 0.5,
            ))
        },
    );

    assert_eq!(element.orders.len(), 3);
    assert_eq!(
        element.orders[0].order_type,
        crate::order::OrderType::TransitionWaitingUprightWalkingUpright
    );
    assert_eq!(
        (element.orders[0].target_x, element.orders[0].target_y),
        (0.0, -4.0),
        "actor transition geometry was fixed before soldier drunken post-processing"
    );
    assert_eq!(
        (element.orders[1].target_x, element.orders[1].target_y),
        (3.0, -20.0)
    );
    assert_eq!(
        (element.orders[2].target_x, element.orders[2].target_y),
        (0.0, -40.0)
    );
    assert_ne!(element.orders[1].order_id, element.orders[2].order_id);
}
