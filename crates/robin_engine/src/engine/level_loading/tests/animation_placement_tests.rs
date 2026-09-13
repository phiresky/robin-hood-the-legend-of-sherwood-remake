use super::apply_animation_sprite_placement;
use crate::coordinates::{MapPoint, SpriteAnchor};
use crate::level_data::RawSpriteRef;
use crate::sprite::Sprite;

#[test]
fn animation_position_uses_top_left_center_and_authored_elevation() {
    let mut sprite = Sprite {
        center: SpriteAnchor::new(12.0, 18.0),
        ..Default::default()
    };
    let raw = RawSpriteRef {
        frame_profile_name: String::new(),
        profile_name: String::new(),
        position_x: 100,
        position_y: 200,
        elevation: 30,
    };

    apply_animation_sprite_placement(&mut sprite, &raw);

    assert_eq!(
        sprite.position_iface.map_position(),
        MapPoint::new(112.0, 218.0)
    );
    assert_eq!(
        sprite.position_iface.get_position(),
        crate::coordinates::WorldPoint3D::new(112.0, 248.0, 30.0)
    );
    assert_eq!(
        sprite.position_iface.map_position().x - sprite.center.x,
        raw.position_x as f32
    );
    assert_eq!(
        sprite.position_iface.map_position().y - sprite.center.y,
        raw.position_y as f32
    );
}
