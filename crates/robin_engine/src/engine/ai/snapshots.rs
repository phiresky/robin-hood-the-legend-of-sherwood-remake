//! Per-entry live optical inputs and archer/shield-link reconciliation.

use super::*;
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Entity, EntityId};
use serde::{Deserialize, Serialize};

/// Enemy archer detection is exactly whether a bow is present.
/// A loaded bow remains a bow even when its normal-shot range is zero.
pub(super) fn is_archer_from_bow(bow: Option<&crate::profiles::BowProfile>) -> bool {
    bow.is_some()
}

/// Geometry and gates for one live human lookup, consumed before the next entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct HumanDetectionInput {
    pub(super) position: MapPoint,
    /// Original-game ground position, i.e. stored world-space X/Y. This is
    /// distinct from projected map position whenever ground Z is non-zero.
    pub(super) ground_position: GroundPoint,
    pub(super) sector: Option<crate::position_interface::SectorHandle>,
    pub(super) layer: u16,
    /// Detection point in world space, taken verbatim as the endpoint
    /// of opaque-reachability queries.
    pub(super) detection_point: crate::coordinates::WorldPoint3D,
    pub(super) posture: crate::element::Posture,
    /// 16-sector facing.  Used for the `LeaningOut` arm of
    /// `compute_detection_point`: the detection point projects
    /// `direction × 40` forward.
    pub(super) direction: i16,
    pub(super) action_state: crate::element::ActionState,
    pub(super) building_sector: Option<crate::position_interface::SectorHandle>,
    /// Canonical human death state. MissedFriend and
    /// Beggar reject dead targets before their per-type cadence decision.
    pub(super) dead: bool,
    pub(super) unconscious: bool,
    pub(super) active: bool,
    pub(super) is_pc: bool,
    /// `is_able_to_help`: alive, conscious, not in a few
    /// state-machine arms that mean "busy with current task".
    /// Used to gate the Friend pass.
    pub(super) able_to_help: bool,
    /// Whether the target is mid-door-pass.  Used by the
    /// same-building visibility short-circuit.
    pub(super) passing_door: bool,
    /// `pc.guard.is_some()`.  Only meaningful for PCs (false for
    /// soldiers / civilians / non-PC entities).  Used by
    /// predetection handling to suppress shadow events for
    /// already-guarded PCs.
    pub(super) guarded: bool,
    /// The projection-obstacle this human is currently standing
    /// on (e.g. a roof, ledge, balcony, or tree platform).
    /// Threaded into the per-target `compute_view_radius` re-call
    /// inside `run_human_detectable_pass` so detection radius
    /// accounts for the target's elevation in night/fog.
    pub(super) obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

/// Geometry for one live object lookup, consumed by its visibility calculation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct ObjectDetectionInput {
    pub(super) position: MapPoint,
    /// Original-game ground position used by the outer detection-refresh box.
    pub(super) ground_position: GroundPoint,
    /// Original-game object point after the detection path raises Z by one.
    pub(super) world_position: crate::coordinates::WorldPoint3D,
    pub(super) belongs_to_beggar: bool,
}

fn object_detection_world_position(
    position: crate::coordinates::WorldPoint3D,
) -> crate::coordinates::WorldPoint3D {
    crate::coordinates::WorldPoint3D::new(position.x, position.y, position.z + 1.0)
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

/// Read the current entry's geometry before borrowing the observer to update
/// its detection latches. No target data survives this entry.
pub(super) fn human_detection_input(
    entities: &crate::entities::Entities,
    id: EntityId,
    grid: &crate::fast_find_grid::FastFindGrid,
) -> Option<HumanDetectionInput> {
    let entity = entities.get(id)?;
    let element = entity.element_data();
    let human = entity
        .human_data()
        .unwrap_or_else(|| panic!("human detectable target {id:?} has no human data"));
    let actor = entity
        .actor_data()
        .unwrap_or_else(|| panic!("human detectable target {id:?} has no actor data"));
    let position = element.position_map();
    let stored_world = element.position();
    let posture = element.posture();
    let direction = element.direction();
    let is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
    let building_sector = element.sector().filter(|&sector| {
        let Some(grid_sector) =
            crate::engine::movement::grid_sector_for_position_handle(&grid.level, sector)
        else {
            return false;
        };
        assert_eq!(
            grid_sector.sector_number,
            crate::sector::SectorNumber::new(i16::from(sector)),
            "exact sector arena identity disagrees with its public number",
        );
        grid_sector.sector_type.is_building()
    });
    // A carried body's stored point retains its own obstacle elevation.
    let able_to_help = matches!(entity, Entity::Soldier(s) if
        crate::ai_enemy::soldier_is_able_to_help_state(
            element.active && !s.human.unconscious && s.npc.life_points > 0,
            s.npc.ai_state(),
            s.npc.ai_substate(),
        )
    );
    Some(HumanDetectionInput {
        position,
        ground_position: GroundPoint::from_map_and_z(position, stored_world.z),
        sector: element.sector(),
        layer: element.layer(),
        detection_point: crate::stealth::detection_point_world(
            stored_world,
            posture,
            direction,
            is_rider,
        ),
        posture,
        direction,
        action_state: actor.action_state,
        building_sector,
        dead: entity.is_dead(),
        unconscious: human.unconscious,
        active: element.active,
        is_pc: matches!(entity, Entity::Pc(_)),
        able_to_help,
        passing_door: actor.active_door_pass.is_some(),
        guarded: matches!(entity, Entity::Pc(pc) if pc.pc.guard.is_some()),
        obstacle_idx: element.obstacle_index(),
    })
}

pub(super) fn is_live_beggar(entity: &Entity) -> bool {
    match entity {
        Entity::Civilian(c) => {
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
        }
        _ => entity.element_data().posture() == crate::element::Posture::SimulatingBeggar,
    }
}

/// Read only the object currently being scanned.
pub(super) fn object_detection_input(
    entities: &crate::entities::Entities,
    id: EntityId,
) -> Option<ObjectDetectionInput> {
    let entity = entities.get(id)?;
    let element = entity.element_data();
    let world_position = object_detection_world_position(element.position());
    Some(ObjectDetectionInput {
        position: element.position_map(),
        ground_position: GroundPoint::new(world_position.x, world_position.y),
        world_position,
        belongs_to_beggar: entity
            .object_data()
            .unwrap_or_else(|| panic!("object detectable target {id:?} has no object data"))
            .belongs_to_beggar,
    })
}

#[cfg(test)]
mod tests {
    use super::{is_archer_from_bow, object_detection_world_position};

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

    #[test]
    fn object_detection_raises_the_ray_above_the_stored_position() {
        assert_eq!(
            object_detection_world_position(crate::coordinates::WorldPoint3D::new(10.0, 25.0, 7.0)),
            crate::coordinates::WorldPoint3D::new(10.0, 25.0, 8.0)
        );
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
