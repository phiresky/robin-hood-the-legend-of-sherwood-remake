//! Archer/shield-link reconciliation and live actor classification.

use super::*;
use crate::element::{Entity, EntityId};

/// Enemy archer detection is exactly whether a bow is present.
/// A loaded bow remains a bow even when its normal-shot range is zero.
pub(super) fn is_archer_from_bow(bow: Option<&crate::profiles::BowProfile>) -> bool {
    bow.is_some()
}

impl EngineInner {
    #[cfg(test)]
    pub(crate) fn ai_pc_snapshot_ids_for_test(&mut self, _assets: &LevelAssets) -> Vec<EntityId> {
        self.world.original_pc_registry().to_vec()
    }

    /// Publish reverse archer links at the established detection-capture point.
    /// Read every claimant before writing: inactive reciprocal links refer to
    /// the pre-refresh state, and the last eligible claimant in registry order wins.
    pub(super) fn refresh_archer_shield_links(&mut self) {
        let mut reverse = std::collections::HashMap::new();
        let mut active_owners = Vec::new();
        for (archer_id, archer) in self.world.entities.soldiers() {
            if archer.human.unconscious {
                continue;
            }
            let ai = archer.npc.ai_brain.enemy().unwrap_or_else(|| {
                panic!("conscious soldier {archer_id:?} has no EnemyAi during shield-link refresh")
            });
            if archer.element.active {
                active_owners.push(EntityId::from(archer_id));
            }
            let Some(shield_handle) = ai.shield_bearer_before_me else {
                continue;
            };
            let shield = self
                .world
                .entities
                .id_at_legacy_slot(shield_handle.get())
                .and_then(|id| self.world.entities.get(id));
            let Some(Entity::Soldier(shield)) = shield.filter(|entity| !entity.is_unconscious())
            else {
                tracing::warn!(
                    archer = EntityId::from(archer_id).index(),
                    shield_bearer = shield_handle.get(),
                    "shield-bearer relationship points outside the conscious soldier registry"
                );
                continue;
            };
            let stored_archer = shield
                .npc
                .ai_brain
                .enemy()
                .expect("conscious shield bearer requires EnemyAi")
                .archer_behind_me;
            let archer_handle = crate::ai::AiEntityHandle::new(EntityId::from(archer_id).index());
            if archer.element.active || stored_archer == Some(archer_handle) {
                reverse.insert(shield_handle, archer_handle);
            } else {
                tracing::warn!(
                    archer = archer_handle.get(),
                    shield_bearer = shield_handle.get(),
                    ?stored_archer,
                    "ignoring stale one-sided inactive archer relationship"
                );
            }
        }
        for id in active_owners {
            self.world
                .entities
                .expect_entity_mut(id, format_args!("shield-link owner"))
                .enemy_ai_mut()
                .expect("active conscious soldier requires EnemyAi")
                .archer_behind_me = reverse.remove(&crate::ai::AiEntityHandle::new(id.index()));
        }
    }
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
    fn shield_link_refresh_preserves_claim_order_and_inactive_reciprocity() {
        use crate::ai::AiEntityHandle;
        use crate::element::Camp;
        use crate::engine::test_support::actors::make_test_ai_soldier;

        // (first active, last active, last unconscious, shield active,
        //  shield unconscious, stored claimant, expected claimant).
        let cases = [
            (
                "last active claimant wins",
                (true, true, false, true, false, None, Some(1)),
            ),
            (
                "reciprocal inactive claimant wins",
                (true, false, false, true, false, Some(1), Some(1)),
            ),
            (
                "stale inactive claimant cannot displace first",
                (true, false, false, true, false, Some(0), Some(0)),
            ),
            (
                "inactive one-sided links are cleared",
                (false, false, false, true, false, None, None),
            ),
            (
                "inactive shield is not overwritten",
                (true, true, false, false, false, Some(0), Some(0)),
            ),
            (
                "unconscious claimant is excluded",
                (true, true, true, true, false, Some(1), Some(0)),
            ),
            (
                "unconscious shield is not overwritten",
                (true, true, false, true, true, Some(0), Some(0)),
            ),
        ];
        for (
            name,
            (
                first_active,
                last_active,
                last_unconscious,
                shield_active,
                shield_unconscious,
                stored,
                expected,
            ),
        ) in cases
        {
            let mut engine = crate::engine::EngineInner::new();
            let shield = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let first = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let last = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let handles = [
                AiEntityHandle::new(first.index()),
                AiEntityHandle::new(last.index()),
            ];
            for (id, active, unconscious) in [
                (shield, shield_active, shield_unconscious),
                (first, first_active, false),
                (last, last_active, last_unconscious),
            ] {
                let entity = engine.get_entity_mut(id).unwrap();
                entity.element_data_mut().active = active;
                entity.human_data_mut().unwrap().unconscious = unconscious;
            }
            for id in [first, last] {
                engine
                    .get_entity_mut(id)
                    .unwrap()
                    .enemy_ai_mut()
                    .unwrap()
                    .shield_bearer_before_me = Some(AiEntityHandle::new(shield.index()));
            }
            engine
                .get_entity_mut(shield)
                .unwrap()
                .enemy_ai_mut()
                .unwrap()
                .archer_behind_me = stored.map(|index: usize| handles[index]);

            engine.refresh_archer_shield_links();

            assert_eq!(
                engine
                    .get_entity(shield)
                    .unwrap()
                    .enemy_ai()
                    .unwrap()
                    .archer_behind_me,
                expected.map(|index| handles[index]),
                "{name}"
            );
            assert_eq!(
                engine
                    .get_entity(first)
                    .unwrap()
                    .enemy_ai()
                    .unwrap()
                    .shield_bearer_before_me,
                Some(AiEntityHandle::new(shield.index())),
                "refresh must preserve forward links: {name}"
            );
        }
    }

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
        let element = engine
            .get_entity_mut(target)
            .expect("test PC exists")
            .element_data_mut();
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
