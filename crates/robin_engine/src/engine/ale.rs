//! Ground-ale bottle spawn.
//!
//! The in-world bottle left behind by a `DropAle` command.  Distinct
//! from pre-placed mission bonuses of `BonusAle` type — those spawn
//! through the PARB chunk loader with the `BONUS_Ale` sprite, whereas
//! an ale dropped by a PC carries the *accessory* `ObjectType::Ale`
//! variant (sprite "ACCESSORIES_Ale", animation `OBJECT_LYING`).
//!
//! The `Command::DropAle` dispatcher lives in the action-dispatch
//! layer; this helper is the receiver that materialises the bottle.

use super::{EngineInner, LevelAssets};
use crate::element::{
    ElementBonus, ElementData, ElementKind, Entity, EntityId, GameMaterial, ObjectData, ObjectType,
};
use crate::order::OrderType;
use crate::position_interface::{ObstacleHandle, SectorHandle};

impl EngineInner {
    /// Spawn a ground-lying ale bottle at the given world anchor.
    ///
    /// Three-step setup:
    ///
    /// 1. Construct an unblipped `OBJECT_OTHERS`-class element with
    ///    `quantity=1`, `taken=false`, animation `WAITING_UPRIGHT`,
    ///    and associated action `Action::Ale`.
    /// 2. Clone the `ObjectType::Ale` accessory sprite prototype
    ///    ("ACCESSORIES_Ale") and force `OrderType::ObjectLying` so
    ///    the bottle renders lying on its side with a fresh frame.
    /// 3. Copy the PC's position / layer / sector / direction /
    ///    obstacle onto the bottle.
    ///
    /// The accessory sprite prototype is preloaded at level load by
    /// [`EngineInner::preload_accessory_sprite_prototypes`] (keyed on
    /// `ObjectType::Ale`) and is cloned here through
    /// [`EngineInner::attach_accessory_sprite`].
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_dropped_ale(
        &mut self,
        assets: &LevelAssets,
        position_map: crate::coordinates::MapPoint,
        layer: u16,
        sector: Option<SectorHandle>,
        direction: i16,
        material: GameMaterial,
        obstacle: Option<ObstacleHandle>,
    ) -> EntityId {
        let mut element = ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            // Always unblipped: unlike a generic bonus, the bottle is
            // never hidden-behind-fog-of-war; it is purely a local prop.
            blipped: false,
            ..Default::default()
        };
        element.sprite.apply_placement(
            position_map,
            layer,
            sector,
            direction,
            material,
            obstacle,
            crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                obstacle,
                assets.static_sight_obstacles.as_slice(),
            ),
        );

        let bonus = Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                quantity: 1,
                object_type: ObjectType::Ale,
                // The pickup will refill the ale slot when taken.
                associated_action: crate::profiles::Action::Ale,
                // Set after the sprite clone so the serialised state
                // matches the row the sprite is on.
                animation: OrderType::ObjectLying,
                ..Default::default()
            },
        });
        let id = self.add_entity(bonus);

        // Clone the preloaded `ACCESSORIES_Ale` sprite so the bottle
        // has real frame data (the inline `Sprite::default()` above
        // carries no frames).  Then force the lying animation row.
        self.attach_accessory_sprite(assets, id);
        if let Some(entity) = self.get_entity_mut(id) {
            let dir = entity.element_data().direction() as u16;
            entity
                .element_data_mut()
                .sprite
                .force_animation(OrderType::ObjectLying, dir);
        }

        id
    }
}
