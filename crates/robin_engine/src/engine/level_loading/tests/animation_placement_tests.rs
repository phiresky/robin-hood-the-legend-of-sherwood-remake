use super::apply_animation_sprite_placement;
use crate::coordinates::{MapPoint, SpriteAnchor};
use crate::level_data::RawSpriteRef;
use crate::sprite::Sprite;

#[test]
#[ignore = "requires SCENERY_TEST_EXPORT_DIR and the native scenery decoding acceptance test"]
fn exported_scenery_spawns_and_advances_frames_without_moving_its_anchor() {
    use super::{EngineInner, LevelAssets, spawn_proto_animation_fx_entities};
    use crate::engine::TickCtx;
    use crate::sprite_script::{MissionResourceEnvironment, SpriteInfo, SpriteScriptor};
    use std::sync::Arc;
    let root = std::path::PathBuf::from(std::env::var("SCENERY_TEST_EXPORT_DIR").unwrap());
    let loaded = crate::level_data::LoadedLevel::hackable_from_json(
        &std::fs::read(root.join("Data/Levels/editor-authored-fixture.level.json")).unwrap(),
    )
    .unwrap();
    let profiles: Vec<(String, SpriteInfo)> =
        serde_json::from_slice(&std::fs::read(root.join("decoded-scenery-profiles.json")).unwrap())
            .unwrap();
    let resources = MissionResourceEnvironment::default()
        .with_parsed_rhs([("Animations/Day/editor-flame.rhs", 0, profiles.as_slice())])
        .unwrap();
    let mut assets = LevelAssets::new();
    assets.sprite_scriptor = Arc::new(SpriteScriptor::with_resources(Arc::new(resources)));
    let mut engine = EngineInner::new();
    assert_eq!(loaded.proto.animations.len(), 1);
    spawn_proto_animation_fx_entities(&mut engine, &mut assets, &loaded.proto.animations);
    let id = engine.world.entities.occupied().next().unwrap().0;
    let raw = &loaded.proto.animations[0];
    let expected = MapPoint::new(
        raw.sprite.position_x as f32 + 4.0,
        raw.sprite.position_y as f32 + 6.0,
    );
    assert_eq!(expected, MapPoint::new(310.0, 310.0));
    assert_eq!(
        engine.ent(id).sprite().position_iface.map_position(),
        expected
    );
    assert_eq!(
        engine.ent(id).sprite().position_iface.get_position().z,
        raw.sprite.elevation as f32
    );
    let sim = crate::sim_rng::test_context();
    let mut frames = Vec::new();
    for _ in 0..20 {
        let sprite = engine.ent(id).sprite();
        frames.push(sprite.bank_id_for(sprite.current_row, sprite.current_frame));
        assert_eq!(sprite.position_iface.map_position(), expected);
        engine.tick_static_entity_hourglass_for(TickCtx::new(&sim, &assets), id);
    }
    assert!(
        frames.contains(&0) && frames.contains(&1),
        "scenery never advanced: {frames:?}"
    );
    assert!(
        frames.windows(2).any(|pair| pair == [1, 0]),
        "scenery never looped: {frames:?}"
    );
    assert_eq!(&frames[..12], &[0, 0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0]);
    engine.ent_mut(id).element_data_mut().active = false;
    let frozen = engine.ent(id).sprite().current_frame;
    for _ in 0..10 {
        engine.tick_static_entity_hourglass_for(TickCtx::new(&sim, &assets), id);
        assert_eq!(engine.ent(id).sprite().current_frame, frozen);
    }
}

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

#[test]
#[ignore = "requires SCENERY_CONFLICT_EXPORT_DIR and native conflicting-bank decoding test"]
fn exported_conflicting_scenery_uses_separate_runtime_profiles() {
    use super::{EngineInner, LevelAssets, spawn_proto_animation_fx_entities};
    use crate::sprite_script::{MissionResourceEnvironment, SpriteInfo, SpriteScriptor};
    use std::sync::Arc;
    let root = std::path::PathBuf::from(std::env::var("SCENERY_CONFLICT_EXPORT_DIR").unwrap());
    let loaded = crate::level_data::LoadedLevel::hackable_from_json(
        &std::fs::read(root.join("Data/Levels/editor-authored-fixture.level.json")).unwrap(),
    )
    .unwrap();
    let banks: Vec<(String, Vec<(String, SpriteInfo)>)> =
        serde_json::from_slice(&std::fs::read(root.join("decoded-scenery-banks.json")).unwrap())
            .unwrap();
    let resources = MissionResourceEnvironment::default()
        .with_parsed_rhs(
            banks
                .iter()
                .map(|(path, profiles)| (path.as_str(), 0, profiles.as_slice())),
        )
        .unwrap();
    let mut assets = LevelAssets::new();
    assets.sprite_scriptor = Arc::new(SpriteScriptor::with_resources(Arc::new(resources)));
    let mut engine = EngineInner::new();
    spawn_proto_animation_fx_entities(&mut engine, &mut assets, &loaded.proto.animations);
    let sprites: Vec<_> = engine
        .world
        .entities
        .occupied()
        .map(|(_, entity)| entity.sprite())
        .collect();
    assert_eq!(sprites.len(), 2);
    assert_ne!(sprites[0].profile_cache_key, sprites[1].profile_cache_key);
    for raw in &loaded.proto.animations {
        let sprite = sprites
            .iter()
            .find(|sprite| sprite.frame_profile_name == raw.sprite.frame_profile_name)
            .unwrap();
        assert_eq!(
            sprite.position_iface.map_position(),
            MapPoint::new(
                raw.sprite.position_x as f32 + 4.,
                raw.sprite.position_y as f32 + 6.
            )
        );
        assert_eq!(sprite.current_scripts()[0].delays, [2, 4]);
    }
}
