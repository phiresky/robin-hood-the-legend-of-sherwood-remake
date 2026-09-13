use super::{mobile_sprite_map_position, prime_mission_start_sprite};
use crate::coordinates::{MapPoint, SpriteAnchor};
use crate::order::OrderType;
use crate::sprite::Sprite;
use crate::sprite_script::UNMAPPED;
use std::sync::Arc;

#[test]
fn primes_authored_action_and_direction_before_first_tick() {
    let action = OrderType::WaitingUprightBored;
    let mut conversion = vec![UNMAPPED; action as usize + 1];
    conversion[action as usize] = 32;
    let mut sprite = Sprite::new(Arc::new(Vec::new()), Arc::new(conversion));

    prime_mission_start_sprite(&mut sprite, action as u32, 11, "test actor");

    assert_eq!(sprite.current_row, 43);
    assert_eq!(sprite.last_action, action);
    assert_eq!(sprite.current_frame, 0);
}

#[test]
fn mobile_animation_position_adds_sprite_center_before_waypoint() {
    let position = mobile_sprite_map_position(
        -70,
        -71,
        SpriteAnchor::new(70.0, 71.0),
        MapPoint::new(772.0, 722.0),
    );

    assert_eq!(position, MapPoint::new(772.0, 722.0));
}
