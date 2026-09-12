//! Shared scenario inputs; actor construction lives in the crate-wide fixtures.
use crate::coordinates::{MapVec, SpriteFrameOffset};
use crate::element::EntityId;
use crate::engine::EngineInner;
pub(in crate::engine) use crate::engine::test_support::actors::{
    make_test_ai_soldier, make_test_civilian, make_test_pc, make_test_soldier,
};
use crate::order::OrderType;
use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

pub(super) fn assets_with_test_pc_profile() -> super::LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles
        .characters
        .push(crate::profiles::CharacterProfile::default());
    super::LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..super::LevelAssets::new()
    }
}

pub(super) fn bind_walking_sprite(
    engine: &mut EngineInner,
    entity_id: EntityId,
    anti_collision: bool,
) {
    let action = OrderType::WalkingUpright;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 20.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 20,
        frame_ids: vec![1],
        delays: vec![0],
        distances: vec![20],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );

    let element = engine
        .get_entity_mut(entity_id)
        .expect("movement fixture actor exists")
        .element_data_mut();
    let position = element.position_map();
    let sector = element.sector();
    sprite.position_iface.set_sector(sector);
    sprite.position_iface.set_anti_collision_on(anti_collision);
    sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            MapVec::new(-2.0, -2.0),
            MapVec::new(2.0, 2.0),
        ));
    element.sprite = sprite;
    element.set_position_map(position);
}
