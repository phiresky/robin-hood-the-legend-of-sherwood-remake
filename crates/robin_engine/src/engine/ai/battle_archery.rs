//! Live archery-path selection and ammunition resupply.

use super::*;
use crate::ai::{
    AiSpeechAttempt, AiState, Decision, EmoticonType, GotoFlags, Position, Remark, Substate,
};
use crate::ai_enemy::{AiMapVec, archer};
use crate::sim_rng::SimulationContext;
use std::ops::ControlFlow;

impl EngineInner {
    pub(in crate::engine) fn choose_ai_good_shooting_point(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let target = self
            .ai(owner, "shooting point target")
            .primary_target
            .expect("shooting point selection requires a target");
        let target = self.expect_human_id_for_ai_handle(target.get(), "shooting point target");
        let enemy_position = self.live_ai_position(target);
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("release shooting point"))
            .set_my_shooting_point(&mut self.ai.global, None);
        let Some(sector) = self.ai.global.archery_sectors.iter().position(|sector| {
            !sector.is_full() && sector.is_inside(&enemy_position, enemy_position.level)
        }) else {
            return false;
        };
        let position = self.live_ai_position(owner);
        let mut entry = None;
        let mut shooting = None;
        let mut minimum_entry = u32::MAX;
        let mut minimum_shooting = u32::MAX;
        for (index, point) in self.ai.global.archery_sectors[sector]
            .points
            .iter()
            .enumerate()
        {
            if self.ai_archer_is_too_near_to_enemy(assets, owner, point.position, target) {
                return false;
            }
            let mut distance =
                (position.map_point() - point.position.map_point()).square_norm() as u32;
            if position.sector.map(u16::from) != Some(u16::from(point.sector_index)) {
                distance = distance.wrapping_add(10_000);
            }
            if !point.is_shooting_point && distance < minimum_entry {
                minimum_entry = distance;
                entry = Some(index);
            }
            if point.is_shooting_point && point.owner.is_none() && distance < minimum_shooting {
                minimum_shooting = distance;
                shooting = Some(index);
            }
        }
        let Some(shooting) = shooting else {
            return false;
        };
        let zone = &self.ai.global.archery_sectors[sector];
        let first = zone.index_first_shooting_point.map_or(u16::MAX, u16::from);
        let last = zone.index_last_shooting_point.map_or(0, u16::from);
        let (index, increment, reserve) = match entry {
            Some(entry) if (entry as u16) < first => (entry as u16, 1, false),
            Some(entry) if (entry as u16) > last => (entry as u16, -1, false),
            Some(entry) if entry < shooting => ((shooting as u16).wrapping_sub(1), 1, true),
            Some(_) => ((shooting as u16).wrapping_add(1), -1, true),
            // TODO: enforce an entry point when validating authored archery paths.
            None => (shooting as u16, 1, true),
        };
        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("shooting path selection"));
        ai.my_archery_sector_index = sector as u16;
        ai.my_archery_point_index = crate::sector::ArcheryPointIdx(index);
        ai.my_archery_point_increment = increment;
        if reserve {
            ai.set_my_shooting_point(&mut self.ai.global, Some((sector as u16, shooting as u16)));
        }
        ai.set_my_archery_sector(&mut self.ai.global, Some(sector as u16));
        true
    }

    pub(in crate::engine) fn ai_archer_is_too_near_to_enemy(
        &self,
        _assets: &LevelAssets,
        owner: EntityId,
        position: Position,
        target: EntityId,
    ) -> bool {
        let entity = self.expect_entity(owner, "archer proximity owner");
        let ai = self.enemy_ai(owner, "archer proximity");
        if ai.shield_bearer_before_me.is_some()
            || (entity.camp() == Camp::Royalists
                && self.world.weather.is_forest_level
                && !entity.soldier_data().expect("archer is soldier").rider)
        {
            return false;
        }
        let enemy = self.expect_entity(target, "archer proximity target");
        let vector = position.map_point() - self.live_ai_position(target).map_point();
        let relative =
            (i32::from(vector.sector_with_aspect(crate::position_interface::ASPECT_RATIO))
                - i32::from(enemy.element_data().direction()))
            .rem_euclid(16);
        let action = enemy
            .actor_data()
            .expect("archer target is actor")
            .action_state;
        let fast = matches!(
            action,
            crate::element::ActionState::MovingFast | crate::element::ActionState::MovingFastSword
        );
        let approaching = matches!(
            action,
            crate::element::ActionState::Moving
                | crate::element::ActionState::MovingShield
                | crate::element::ActionState::MovingSword
        );
        let distance = match relative {
            0 if fast => archer::MIN_DISTANCE_ENEMY_HEAD_ON_ATTACK,
            0 if approaching => archer::MIN_DISTANCE_ENEMY_APPROACHING_FAST,
            0 => archer::MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY,
            1 | 15 if fast => archer::MIN_DISTANCE_ENEMY_APPROACHING_FAST,
            1 | 15 | 2 | 14 if approaching => archer::MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY,
            2 | 14 if fast => archer::MIN_DISTANCE_ENEMY_APPROACHING,
            1 | 15 | 2 | 14 | 3 | 4 | 12 | 13 => archer::MIN_DISTANCE_ENEMY_PASSING,
            _ => archer::MIN_DISTANCE_ENEMY_LEAVING,
        } as f32;
        crate::position_interface::vector_square_norm_iso(vector.x, vector.y) < distance * distance
    }

    pub(in crate::engine) fn execute_ai_battle_archery_point(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> ControlFlow<bool, Decision> {
        let ai = self.enemy_ai(owner, "archery waypoint");
        let sector = usize::from(ai.my_archery_sector_index);
        let index = usize::from(ai.my_archery_point_index);
        let Some(point) = self
            .ai
            .global
            .archery_sectors
            .get(sector)
            .and_then(|sector| sector.points.get(index))
        else {
            return ControlFlow::Continue(Decision::Shoot);
        };
        if point.owner.is_some() {
            return ControlFlow::Continue(Decision::Shoot);
        }
        let shooting = point.is_shooting_point;
        let primary = ai.base.primary_target;
        let elevation = primary
            .map(|target| {
                self.expect_entity(
                    self.expect_human_id_for_ai_handle(target.get(), "archery elevation"),
                    "archery elevation",
                )
                .position_iface()
                .get_elevation() as u16
            })
            .unwrap_or(0);
        self.enemy_ai_mut(owner, "archery elevation")
            .enemy_had_this_elevation = elevation;
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            if shooting {
                Substate::AttackingArcherRunOnShootingPathFinalSprint
            } else {
                Substate::AttackingArcherRunOnShootingPath
            },
        );
        if shooting {
            self.world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("reserve shooting point"))
                .set_my_shooting_point(&mut self.ai.global, Some((sector as u16, index as u16)));
        }
        let position = self.ai.global.archery_sectors[sector].points[index].position;
        self.duty_go_to(
            sim,
            assets,
            owner,
            position,
            if shooting {
                GotoFlags::RUN
            } else {
                GotoFlags::RUN | GotoFlags::DONT_STOP
            },
        );
        ControlFlow::Break(true)
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_battle_run_for_arrows(
        &mut self,
    ) -> ControlFlow<bool, Decision> {
        let position = self.engine.live_ai_position(self.owner);
        let entity = self.engine.expect_entity(self.owner, "arrow reserve owner");
        let building = self
            .engine
            .entity_building_sector(entity.element_data().sector());
        let raw_sector = entity.element_data().sector();
        let layer = entity.element_data().layer();
        let auth = entity.actor_auth_info();
        let camp = entity.camp();
        let mut nearest = None;
        let mut minimum = u16::MAX;
        for (index, door) in self
            .engine
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
        {
            if door.gate_type != crate::gate::GateType::Door
                || door.door_type != crate::gate::DoorType::Building
                || building.map(u16::from) == Some(u16::from(door.sector_in))
                || !door.is_actor_authorized(
                    true,
                    &auth,
                    self.engine.building_sector_is_authorized(door.sector_in),
                    false,
                )
            {
                continue;
            }
            let building = self
                .engine
                .grid_sector_by_number(door.sector_in)
                .expect("arrow reserve door requires building sector")
                .building_index
                .expect("arrow reserve building identity");
            let building = usize::from(building.get());
            if !self.engine.script_domains.buildings.arrow_reserves[building] {
                continue;
            }
            let distance = crate::ai_enemy::legacy_nearest_door_distance(
                door.point_out.x - position.x,
                door.point_out.y - position.y,
                raw_sector.map(u16::from) != Some(u16::from(door.sector_out)),
                layer != door.layer_out,
            );
            if distance < minimum {
                let dangerous = camp == Camp::Lacklandists
                    && self.engine.script_domains.buildings.occupants[building]
                        .iter()
                        .any(|&handle| {
                            matches!(
                                self.engine.expect_entity_id_for_index(
                                    handle as u32,
                                    "arrow reserve occupant"
                                ),
                                EntityId::Pc(_)
                            )
                        });
                if !dangerous {
                    minimum = distance;
                    nearest = Some(index);
                }
            }
        }
        self.execute_ai_speech(AiSpeechAttempt {
            remark: Remark::OutOfAmmunition,
            flags: 0,
        });
        let Some(index) = nearest else {
            return ControlFlow::Continue(Decision::Cassos);
        };
        let target = self
            .engine
            .ai(self.owner, "arrow return target")
            .primary_target;
        let position = self.engine.live_ai_position(
            target
                .map(|target| {
                    self.engine
                        .expect_human_id_for_ai_handle(target.get(), "arrow return target")
                })
                .unwrap_or(self.owner),
        );
        self.engine
            .ai_mut(self.owner, "arrow return position")
            .seek_position = position;
        self.duty_set_state(AiState::Fleeing, Substate::FleeingRunForArrowReserves);
        self.engine
            .ai_mut(self.owner, "arrow reserve emoticon")
            .set_transient_emoticon(EmoticonType::XMark, 100, 0);
        let door = &self.engine.script_domains.interactables.doors[index];
        let mut sector = crate::position_interface::SectorHandle::new(u16::from(door.sector_in))
            .expect("building sector");
        if let Some(arena) = door.sector_in_index {
            sector = sector.with_arena_index(arena);
        }
        let position = Position {
            x: door.point_in.x,
            y: door.point_in.y,
            sector: Some(sector),
            level: door.layer_in,
        };
        self.duty_go_to(position, GotoFlags::RUN);
        ControlFlow::Break(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiEntityHandle, PointArchery, SectorArchery};
    use crate::coordinates::WorldPoint3D;
    use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_pc};

    fn archery_fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        for (id, x) in [(owner, 100.0), (target, 1000.0)] {
            let entity = engine.get_entity_mut(id).unwrap();
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, 100.0, 0.0));
            entity
                .element_data_mut()
                .set_sector(crate::position_interface::SectorHandle::new(1));
        }
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("archery test target"))
            .primary_target = Some(AiEntityHandle::new(target.index()));
        let position = engine.live_ai_position(owner);
        let point = |x, shooting| PointArchery {
            position: Position { x, ..position },
            direction: 0,
            is_shooting_point: shooting,
            sector_index: crate::sector::SectorNumber::new(1),
            owner: None,
        };
        engine.ai.global.archery_sectors.push(SectorArchery {
            points: vec![point(90.0, false), point(120.04, true), point(120.02, true)],
            polygon: vec![(900.0, 0.0), (1100.0, 0.0), (1100.0, 200.0), (900.0, 200.0)],
            layer: 0,
            index_first_shooting_point: Some(crate::sector::ArcheryPointIdx(1)),
            index_last_shooting_point: Some(crate::sector::ArcheryPointIdx(2)),
            num_shooting_points: 2,
            num_owners: 0,
        });
        (engine, LevelAssets::new(), owner, target)
    }

    #[test]
    fn shooting_path_rejects_a_point_claimed_after_selection() {
        let (mut engine, assets, owner, target) = archery_fixture();
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("archery cursor"));
        ai.my_archery_sector_index = 0;
        ai.my_archery_point_index = crate::sector::ArcheryPointIdx(1);
        engine.ai.global.archery_sectors[0].points[1].owner = Some(target);
        assert!(matches!(
            engine.execute_ai_battle_archery_point(&crate::sim_rng::test_context(), &assets, owner),
            ControlFlow::Continue(Decision::Shoot)
        ));
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .all(
                    |sequence| sequence
                        .elements
                        .iter()
                        .all(|element| element.owner != Some(owner)
                            || !matches!(
                                element.data,
                                crate::sequence::SequenceElementData::Movement { .. }
                            ))
                )
        );
    }

    #[test]
    fn shooting_point_search_observes_live_reservations_and_target_position() {
        let (mut engine, assets, owner, target) = archery_fixture();
        engine.ai.global.archery_sectors[0].points[1].owner = Some(target);
        assert!(engine.choose_ai_good_shooting_point(&assets, owner));
        assert_eq!(
            engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("archery path"))
                .my_archery_point_index,
            crate::sector::ArcheryPointIdx(0)
        );
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(2000.0, 100.0, 0.0));
        assert!(!engine.choose_ai_good_shooting_point(&assets, owner));
    }
}
