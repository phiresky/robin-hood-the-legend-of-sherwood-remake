//! Live actor classification and profile queries.

use super::*;
use crate::element::{Entity, EntityId};

/// Enemy archer detection is exactly whether a bow is present.
/// A loaded bow remains a bow even when its normal-shot range is zero.
pub(super) fn is_archer_from_bow(bow: Option<&crate::profiles::BowProfile>) -> bool {
    bow.is_some()
}

pub(super) fn is_live_beggar(entity: &Entity) -> bool {
    match entity {
        Entity::Civilian(c) => {
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
        }
        _ => entity.element_data().posture() == crate::element::Posture::SimulatingBeggar,
    }
}

#[cfg(test)]
mod tests {
    use super::is_archer_from_bow;

    #[test]
    fn owner_boundary_ai_position_recovers_duplicate_public_sector_identity() {
        use crate::coordinates::{MapBBox, MapPoint};
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::sector::{SectorNumber, SectorType};

        let grid_sector = |min, max| GridSector {
            points: vec![
                MapPoint::new(min, min),
                MapPoint::new(max, min),
                MapPoint::new(max, max),
                MapPoint::new(min, max),
            ],
            bounding_box: MapBBox::from_coords(min, min, max, max),
            sector_type: SectorType::MOTION | SectorType::AREA,
            layer: 2,
            sector_number: SectorNumber::new(88),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        };

        let mut engine = crate::engine::EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(300.0, 350.0), 2);
        let exact = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(100.0, 200.0), 2);
        assert_ne!(wrong, exact);

        let target = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: {
                let mut initial_element = crate::element::ElementData::from_initial_posture(
                    crate::element::Posture::Upright,
                );
                initial_element.kind = crate::element::ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }));
        let element = engine.elem_mut(target);
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));

        let position = engine.live_ai_position(target);
        assert_eq!(
            position.sector.and_then(|sector| sector.arena_index()),
            SectorIndex::new(exact)
        );
    }

    #[test]
    fn bow_presence_defines_archer_even_with_zero_normal_range() {
        let bow = crate::profiles::BowProfile::default();
        assert_eq!(bow.normal_shoot.range, 0);
        assert!(is_archer_from_bow(Some(&bow)));
        assert!(!is_archer_from_bow(None));
    }
}

impl EngineInner {
    pub(super) fn soldier_profile_facts<'a>(
        &self,
        assets: &'a LevelAssets,
        s: &crate::element::ActorSoldier,
        id: EntityId,
    ) -> (
        &'a crate::profiles::SoldierProfile,
        u16,
        Option<&'a crate::profiles::BowProfile>,
    ) {
        let soldier_profile = assets
            .profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .unwrap_or_else(|| {
                panic!(
                    "soldier {} requires missing soldier profile {}",
                    id.index(),
                    u32::from(s.soldier.soldier_profile_index)
                )
            });
        let fighting_ability = {
            let base = soldier_profile.fighting;
            if self.is_hostile_to_player_camp(s.soldier.cached_camp) {
                let diff = self.control.sim_config.difficulty;
                diff.rules().enemy_fighting(base, 100)
            } else {
                base
            }
        };
        // Enemy archer detection is exactly
        // the actor having a bow. Weapon initialization creates the
        // bow whenever the one-based shooting-weapon id is non-zero;
        // the bow profile's ranges do not participate in identity.
        let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
            None
        } else {
            Some(
                assets
                    .profile_manager
                    .get_bow(soldier_profile.shooting_weapon_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "soldier {} requires missing bow profile {}",
                            id.index(),
                            soldier_profile.shooting_weapon_id
                        )
                    }),
            )
        };

        (soldier_profile, fighting_ability, bow_profile)
    }
}
