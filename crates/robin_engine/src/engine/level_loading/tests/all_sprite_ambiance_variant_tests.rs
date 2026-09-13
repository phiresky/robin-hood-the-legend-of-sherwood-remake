use super::EngineInner;
use crate::element::{
    ElementBonus, ElementData, ElementFx, ElementKind, ElementTarget, Entity, FxData, ObjectData,
    ObjectType, TargetData,
};
use crate::engine::Ambiance;
use crate::sprite_variant::SpriteVariant;

fn bonus() -> Entity {
    Entity::Bonus(ElementBonus {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectBonus;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::BonusApple,
            ..Default::default()
        },
    })
}

fn fx(mobile_index: Option<u16>) -> Entity {
    Entity::Fx(ElementFx {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element
        },
        fx: FxData {
            mobile_index,
            ..Default::default()
        },
    })
}

fn target() -> Entity {
    Entity::Target(ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: FxData::default(),
        target: TargetData::default(),
    })
}

#[test]
fn all_sprite_ambiance_tints_only_day_based_sprites() {
    let mut engine = EngineInner::new();
    engine.world.weather.ambiance = Ambiance::Fog;

    assert_eq!(
        engine.resolve_render_variant(&bonus(), false),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&bonus(), true),
        SpriteVariant::Fog
    );
    assert_eq!(
        engine.resolve_render_variant(&fx(None), true),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&target(), true),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&fx(Some(0)), true),
        SpriteVariant::Fog
    );
    engine.world.weather.ambiance = Ambiance::Night;
    assert_eq!(
        engine.resolve_render_variant(&bonus(), false),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&bonus(), true),
        SpriteVariant::Night
    );
    assert_eq!(
        engine.resolve_render_variant(&fx(None), true),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&target(), true),
        SpriteVariant::Day
    );
    assert_eq!(
        engine.resolve_render_variant(&fx(Some(0)), true),
        SpriteVariant::Night
    );
}
