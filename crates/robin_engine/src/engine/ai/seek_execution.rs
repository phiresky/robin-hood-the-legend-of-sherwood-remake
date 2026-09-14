//! Live area-search setup and seek-point advancement.
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, EmoticonType, GotoFlags, Position, SeekPoint, Substate,
};
use crate::ai_enemy::{EnemyAi, SeekAreaSpec, SeekFlags, UNDEFINED_DIRECTION, task_priority};
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    fn seek_enemy(&self, owner: EntityId) -> &EnemyAi {
        self.expect_entity(owner, "area-search owner")
            .enemy_ai()
            .expect("area search requires enemy AI")
    }
    fn seek_enemy_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.world
            .entities
            .expect_entity_mut(owner, format_args!("area-search owner"))
            .enemy_ai_mut()
            .expect("area search requires enemy AI")
    }
    fn seek_point_mut(&mut self, owner: EntityId, id: u16) -> &mut SeekPoint {
        match id {
            1111 => self
                .seek_enemy_mut(owner)
                .personal_seek_point_1
                .as_mut()
                .expect("missing first personal seek point"),
            2222 => self
                .seek_enemy_mut(owner)
                .personal_seek_point_2
                .as_mut()
                .expect("missing second personal seek point"),
            _ => self
                .ai
                .global
                .seek_points
                .get_mut(usize::from(id))
                .expect("missing global seek point"),
        }
    }
    fn resolve_live_seek_center(&self, owner: EntityId, mut center: Position) -> Position {
        if center.sector.is_some_and(|s| s.arena_index().is_some()) {
            return center;
        }
        let reference = self.live_ai_position(owner).map_point();
        let hit = self
            .world
            .fast_grid
            .get_sector(center.map_point(), reference, center.level);
        if let Some(sector) = hit.sector_handle()
            && center.sector.is_none_or(|authored| authored == sector)
        {
            center.sector = Some(sector);
        }
        center
    }

    pub(in crate::engine) fn execute_ai_seek_area(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
        standard_radius: u16,
        flags: SeekFlags,
        seek_direction: u16,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let center = self.resolve_live_seek_center(owner, center);
        self.seek_enemy_mut(owner).base.stop_all();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.seek_enemy_mut(owner).base.outbox.actor.set_unfocus();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        if self.is_player_aligned_camp(self.expect_entity(owner, "seek camp").camp())
            || self.seek_enemy(owner).company_number == 100
        {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        if !flags.contains(SeekFlags::CHARLY_SEEK) {
            self.seek_enemy_mut(owner).base.set_checkpoint_charly(None);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
        self.seek_enemy_mut(owner).current_task_priority = task_priority::SEEKING;
        if self.execute_seek_other_bodies(sim, assets, owner) {
            return;
        }

        let hostile =
            self.is_hostile_to_player_camp(self.expect_entity(owner, "seek IQ camp").camp());
        let iq = self
            .seek_enemy(owner)
            .iq_for_difficulty(self.control.sim_config.difficulty, hostile);
        if i32::from(iq) >= parameters_ai::CHECK_BEGGAR_MIN_IQ
            && !self.seek_enemy(owner).combat_trainer
        {
            let mut beggars: Vec<_> = self
                .world
                .entities
                .occupied()
                .filter_map(|(id, entity)| {
                    let beggar = match entity {
                        Entity::Civilian(c) => {
                            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
                        }
                        Entity::Pc(_) | Entity::Soldier(_) => {
                            entity.element_data().posture()
                                == crate::element::Posture::SimulatingBeggar
                        }
                        _ => false,
                    };
                    beggar.then_some(id)
                })
                .collect();
            beggars.sort_unstable_by_key(|id| self.world.original_creation_order(*id));
            let ai = self.seek_enemy_mut(owner);
            ai.base
                .outbox
                .actor
                .delete_detectable_type(crate::element::DetectableType::Beggar);
            ai.beggar_to_examine = None;
            for id in beggars {
                ai.base
                    .outbox
                    .actor
                    .add_detectable((id, crate::element::DetectableType::Beggar));
            }
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }

        let ai = self.seek_enemy_mut(owner);
        ai.seek_flags = flags;
        ai.seek_center = center;
        ai.my_seek_points.clear();
        let spec = SeekAreaSpec {
            center,
            standard_radius,
            flags,
            seek_direction,
        };
        let frame = self.control.frame_counter;
        let creation_order = Some(self.world.original_creation_order(owner));
        if standard_radius > 0 && !self.seek_enemy(owner).combat_trainer {
            let position = self.live_ai_position(owner);
            let mut seeking_friends = 0usize;
            let mut clears_help = false;
            for (id, soldier) in self.world.entities.soldiers() {
                let id = EntityId::Soldier(id);
                if id == owner {
                    continue;
                }
                let Some(ai) = soldier.npc.ai_brain.enemy() else {
                    continue;
                };
                if ai.base.view_alert_status == crate::ai::AlertLevel::Green {
                    continue;
                }
                let friend = self.live_ai_position(id);
                let dx = position.x - friend.x;
                let dy = position.y - friend.y;
                if dx * dx + dy * dy >= 500.0 * 500.0 {
                    continue;
                }
                seeking_friends += 1;
                clears_help |= ai.base.current_substate.is_seek_area()
                    && ai.seek_flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER);
            }
            let ai = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("seek point selection"))
                .enemy_ai_mut()
                .expect("seek point selection requires enemy AI");
            ai.append_global_area_seek_points(
                sim,
                frame,
                creation_order,
                seeking_friends,
                clears_help,
                spec,
                &mut self.ai.global,
            );
        } else {
            debug_assert!(flags.intersects(SeekFlags::LOCATION_FIRST | SeekFlags::LOCATION_END));
        }
        if flags.contains(SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE) {
            let adjusted = self.live_seek_door_center(
                owner,
                self.seek_enemy(owner).seek_center,
                seek_direction,
            );
            self.seek_enemy_mut(owner).seek_center = adjusted;
        }
        let ai = self
            .world
            .entities
            .expect_entity_mut(owner, format_args!("personal seek points"))
            .enemy_ai_mut()
            .expect("personal seek points require enemy AI");
        ai.append_personal_area_seek_points(sim, spec, frame, creation_order);
        ai.actual_seek_point = None;
        assert!(
            !ai.my_seek_points.is_empty(),
            "area search must produce a seek point"
        );

        let building = self.entity_building_sector(
            self.expect_entity(owner, "seek building")
                .element_data()
                .sector(),
        );
        if building.is_none() {
            self.execute_ai_seek_next_point(sim, assets, owner);
        } else {
            self.seek_enemy_mut(owner)
                .seek_point_view_directions
                .clear();
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingSeekpointWatchingSidewards,
            );
            let frame = self.control.frame_counter;
            self.seek_enemy_mut(owner).base.launch_timer(3, frame);
        }
    }

    fn live_seek_door_center(&self, owner: EntityId, center: Position, direction: u16) -> Position {
        let mut nearest = center;
        let mut distance = parameters_ai::MAX_SEARCH_ENEMY_BEHIND_DOOR_DISTANCE;
        let entity = self.expect_entity(owner, "seek door owner");
        let building = self.entity_building_sector(entity.element_data().sector());
        let auth = entity.actor_auth_info();
        for door in &self.script_domains.interactables.doors {
            if door.door_type != crate::gate::DoorType::Building {
                continue;
            }
            let Some(sector) = center.sector else {
                continue;
            };
            if u16::from(door.sector_out) != u16::from(sector) {
                continue;
            }
            if let Some(index) = sector.arena_index() {
                assert!(
                    door.sector_out_index.is_some(),
                    "seek door lacks exterior arena identity"
                );
                if door.sector_out_index != Some(index) {
                    continue;
                }
            }
            if building.map(u16::from) == Some(u16::from(door.sector_in)) {
                continue;
            }
            if !door.is_actor_authorized(
                true,
                &auth,
                self.building_sector_is_authorized(door.sector_in),
                false,
            ) {
                continue;
            }
            let dx = door.point_out.x - center.x;
            let dy = door.point_out.y - center.y;
            let door_direction =
                crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16;
            let delta = door_direction.wrapping_add(16).wrapping_sub(direction) & 15;
            if matches!(delta, 15 | 0 | 1) {
                let candidate = dx.abs().max(dy.abs()) as u16;
                if candidate < distance {
                    distance = candidate;
                    nearest = Position {
                        x: door.point_in.x,
                        y: door.point_in.y,
                        level: door.layer_in,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_in,
                        ))
                        .map(|sector| {
                            sector.with_arena_index(
                                door.sector_in_index
                                    .expect("seek door lacks interior arena identity"),
                            )
                        }),
                    };
                }
            }
        }
        nearest
    }

    pub(in crate::engine) fn execute_ai_seek_next_point(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        loop {
            if let Some(id) = self.seek_enemy(owner).actual_seek_point {
                self.seek_point_mut(owner, id).locked = false;
            }
            self.seek_enemy_mut(owner).current_task_priority = task_priority::SEEKING;
            if !self.seek_enemy(owner).beggars_to_control.is_empty() {
                let ai = self.seek_enemy_mut(owner);
                let beggar = ai.beggars_to_control.pop().expect("nonempty beggar queue");
                ai.beggar_to_examine = Some(AiEntityHandle::new(beggar));
                ai.base.seek_position = ai
                    .positions_of_beggars_to_control
                    .pop()
                    .expect("beggar position missing");
                let id = self.expect_entity_id_for_index(beggar, "seek beggar identity");
                let civilian = matches!(self.expect_entity(id, "seek beggar"), Entity::Civilian(_));
                self.seek_enemy_mut(owner).beggar_is_npc = civilian;
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Seeking,
                    Substate::SeekingSeekpointApproachingBeggar,
                );
                let position = self.seek_enemy(owner).base.seek_position;
                self.duty_go_near(sim, assets, owner, position, 50, GotoFlags::RUN);
                return;
            }
            if self.seek_enemy(owner).my_seek_points.is_empty() {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                self.execute_finish_exhausted_search(sim, assets, owner);
                return;
            }
            let id = self.seek_enemy_mut(owner).my_seek_points.remove(0);
            self.seek_enemy_mut(owner).actual_seek_point = Some(id);
            if self.seek_point_mut(owner, id).locked {
                continue;
            }
            let frame = self.control.frame_counter;
            let interest = self.seek_point_mut(owner, id).calculate_interest(frame);
            if crate::sim_rng::u8(sim, crate::sim_rng::RngSite::SeekPointAcceptance, 0..100)
                >= interest
            {
                continue;
            }
            let point = self.seek_point_mut(owner, id);
            point.subtract_interest(
                parameters_ai::SEEK_POINT_EXAMINE_DELTA_INTEREST as u8,
                frame,
            );
            point.locked = true;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingSeekpoint,
            );
            self.seek_enemy_mut(owner)
                .base
                .set_emoticon(EmoticonType::QuestionMark);
            let flags = if self
                .seek_enemy(owner)
                .seek_flags
                .contains(SeekFlags::WALKING)
            {
                GotoFlags::empty()
            } else {
                GotoFlags::RUN
            };
            let current = self
                .seek_enemy(owner)
                .actual_seek_point
                .expect("seek callback cleared current seek point");
            let position = self.seek_point_mut(owner, current).position;
            let position = self.resolve_live_seek_center(owner, position);
            self.duty_go_to(sim, assets, owner, position, flags);
            return;
        }
    }

    fn execute_seek_other_bodies(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        loop {
            let Some(&body) = self.seek_enemy(owner).other_bodies_to_examine.first() else {
                return false;
            };
            let id = self.expect_entity_id_for_index(body, "queued seek body");
            let entity = self.expect_entity(id, "queued seek body");
            let human = entity.human_data().expect("queued seek body must be human");
            let in_coma = if let Entity::Pc(pc) = entity {
                let description = pc
                    .pc
                    .campaign_description_index
                    .expect("seek body PC lacks campaign identity");
                self.mission_domain
                    .campaign
                    .characters
                    .get(usize::try_from(description).expect("campaign character index overflow"))
                    .expect("seek body PC campaign identity is invalid")
                    .status
                    .in_coma
            } else {
                false
            };
            let down = entity.human_life_points() <= 0
                || entity.is_unconscious()
                || human.stuck_under_nets_counter > 0
                || in_coma
                || matches!(
                    entity.element_data().posture(),
                    crate::element::Posture::Tied | crate::element::Posture::Carried
                );
            self.seek_enemy_mut(owner).other_bodies_to_examine.remove(0);
            if down {
                self.execute_seek_body(sim, assets, owner, id);
                return true;
            }
        }
    }

    fn execute_seek_body(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        body: EntityId,
    ) {
        if self
            .expect_entity(body, "seek body")
            .human_data()
            .expect("seek body must be human")
            .stuck_under_nets_counter
            > 0
        {
            self.execute_seek_net_victim(sim, assets, owner, body);
            return;
        }
        let position = self.live_ai_position(body);
        let ai = self.seek_enemy_mut(owner);
        ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
        ai.base.seek_position = position;
        ai.base.set_emoticon(EmoticonType::XMark);
        self.duty_set_state(sim, assets, owner, AiState::Seeking, Substate::SeekingBody);
        self.seek_enemy_mut(owner)
            .base
            .outbox
            .actor
            .set_focus(AiEntityHandle::new(body.index()));
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let position = self.seek_enemy(owner).base.seek_position;
        self.duty_go_near(
            sim,
            assets,
            owner,
            position,
            parameters_ai::AI_STOP_BEFORE_BODY_STEPS,
            GotoFlags::RUN,
        );
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner).base.launch_timer(10, frame);
    }

    fn execute_seek_net_victim(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        victim: EntityId,
    ) {
        let position = self.live_ai_position(owner);
        let mut nearest = None;
        let mut distance = f32::INFINITY;
        for (id, net) in self.world.entities.nets() {
            if !net.element.active || !net.net.victims.contains(&victim) {
                continue;
            }
            let p = net.element.position_map();
            let d = (p.x - position.x)
                .abs()
                .max((p.y - position.y).abs() * crate::position_interface::INVERSE_ASPECT_RATIO);
            if d < distance {
                distance = d;
                nearest = Some(EntityId::Net(id));
            }
        }
        let net = nearest.expect("stuck victim has no covering net");
        let ai = self.seek_enemy_mut(owner);
        ai.base.detected_body = Some(AiEntityHandle::new(victim.index()));
        ai.base.interesting_object = Some(AiEntityHandle::new(net.index()));
        let victim_entity = self.expect_entity(victim, "net victim");
        let net_entity = self.expect_entity(net, "covering net");
        let reachable = self.world.fast_grid.is_straight_movement_authorized(
            victim_entity.element_data().position_map(),
            net_entity.element_data().position_map(),
            victim_entity.element_data().layer(),
            self.expect_entity(owner, "net rescuer")
                .position_iface()
                .get_move_box(),
        );
        let (goal, distance) = if reachable {
            let Entity::Net(net_data) = net_entity else {
                unreachable!()
            };
            (
                self.live_ai_position(net),
                if net_data.net.crumpled { 25 } else { 55 },
            )
        } else {
            (self.live_ai_position(victim), 15)
        };
        self.duty_set_state(sim, assets, owner, AiState::Seeking, Substate::SeekingNet);
        self.duty_go_near(sim, assets, owner, goal, distance, GotoFlags::RUN);
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner).base.launch_timer(10, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_search_sector(engine: &mut EngineInner, number: u16, building: bool) -> SectorHandle {
        let mut sector = crate::engine::test_support::square_sector(
            number as i16,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(3000.0, 3000.0),
        );
        if building {
            sector.sector_type |= crate::sector::SectorType::BUILDING;
        }
        let index = engine.world.fast_grid_mut().add_sector(sector, 0);
        SectorHandle::new(number)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap())
    }

    fn search_door(outside: SectorHandle, inside: SectorHandle, x: f32) -> crate::gate::Door {
        crate::gate::Door {
            door_type: crate::gate::DoorType::Building,
            active: true,
            point_out: MapPoint::new(x, 100.0),
            point_in: MapPoint::new(200.0, 200.0),
            sector_out: crate::sector::SectorNumber::new(outside.get() as i16),
            sector_out_index: outside.arena_index(),
            sector_in: crate::sector::SectorNumber::new(inside.get() as i16),
            sector_in_index: inside.arena_index(),
            ..Default::default()
        }
    }

    #[test]
    fn enemy_behind_door_uses_exact_outside_sector_identity() {
        let (mut engine, _, owner) = search_fixture(EnemyAi::new(1), Default::default(), false);
        let outside = add_search_sector(&mut engine, 88, false);
        let duplicate = add_search_sector(&mut engine, 88, false);
        let wrong_inside = add_search_sector(&mut engine, 18, true);
        let correct_inside = add_search_sector(&mut engine, 19, true);
        let mut wrong = search_door(duplicate, wrong_inside, 101.0);
        wrong.point_in = MapPoint::new(900.0, 900.0);
        engine.script_domains.interactables.doors =
            vec![wrong, search_door(outside, correct_inside, 110.0)];
        let center = Position {
            x: 100.0,
            y: 100.0,
            sector: Some(outside),
            level: 0,
        };
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(10.0, 0.0) as u16;
        let exact = engine.live_seek_door_center(owner, center, direction);
        assert_eq!(
            exact.sector.unwrap().arena_index(),
            correct_inside.arena_index()
        );
        assert_eq!(exact.map_point(), MapPoint::new(200.0, 200.0));
        let numeric = Position {
            sector: SectorHandle::new(88),
            ..center
        };
        let numeric = engine.live_seek_door_center(owner, numeric, direction);
        assert_eq!(
            numeric.sector.unwrap().arena_index(),
            wrong_inside.arena_index()
        );
        assert_eq!(numeric.map_point(), MapPoint::new(900.0, 900.0));
    }

    #[test]
    #[should_panic(expected = "seek door lacks exterior arena identity")]
    fn enemy_behind_door_rejects_missing_exact_outside_identity() {
        let (mut engine, _, owner) = search_fixture(EnemyAi::new(1), Default::default(), false);
        let inside = add_search_sector(&mut engine, 18, true);
        let mut door = search_door(test_sector(), inside, 101.0);
        door.sector_out_index = None;
        engine.script_domains.interactables.doors.push(door);
        engine.live_seek_door_center(
            owner,
            Position {
                x: 100.0,
                y: 100.0,
                sector: Some(test_sector()),
                level: 0,
            },
            crate::position_interface::vector_to_sector_0_to_15_iso(1.0, 0.0) as u16,
        );
    }

    #[test]
    fn sectorless_group_seek_center_recovers_original_position_sector() {
        let (engine, _, owner) = search_fixture(EnemyAi::new(1), Default::default(), false);
        let center = Position {
            x: 64.0,
            y: 64.0,
            sector: None,
            level: 0,
        };
        let resolved = engine.resolve_live_seek_center(owner, center);
        assert_eq!(resolved.sector, Some(test_sector()));
        assert_eq!(
            resolved.sector.unwrap().arena_index(),
            test_sector().arena_index()
        );
        let numeric = Position {
            sector: SectorHandle::new(1),
            ..center
        };
        assert_eq!(
            engine
                .resolve_live_seek_center(owner, numeric)
                .sector
                .unwrap()
                .arena_index(),
            test_sector().arena_index()
        );
        let conflicting = Position {
            sector: SectorHandle::new(7),
            ..center
        };
        let resolved = engine.resolve_live_seek_center(owner, conflicting);
        assert_eq!(resolved.sector, conflicting.sector);
        assert!(resolved.sector.unwrap().arena_index().is_none());
    }

    #[test]
    fn find_door_enemy_could_be_behind_applies_every_authorization_gate_before_personal_point() {
        use crate::gate::DoorType;
        for (name, door_type, active, locked, full, rider, accepted) in [
            (
                "authorized",
                DoorType::Building,
                true,
                false,
                false,
                false,
                true,
            ),
            (
                "building type",
                DoorType::Default,
                true,
                false,
                false,
                false,
                false,
            ),
            (
                "active state",
                DoorType::Building,
                false,
                false,
                false,
                false,
                false,
            ),
            (
                "building capacity",
                DoorType::Building,
                true,
                false,
                true,
                false,
                false,
            ),
            ("rider", DoorType::Building, true, false, false, true, false),
            (
                "villain lock",
                DoorType::Building,
                true,
                true,
                false,
                false,
                false,
            ),
        ] {
            let (mut engine, assets, owner) =
                search_fixture(EnemyAi::new(1), Default::default(), true);
            engine.control.frame_counter = 100;
            let outside = add_search_sector(&mut engine, 7, false);
            let inside = add_search_sector(&mut engine, 8, true);
            let index = inside.arena_index().unwrap();
            engine.world.fast_grid_mut().level_mut().sectors[index.get() as usize].building_index =
                crate::sector::BuildingIdx::new(0);
            let handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
            engine.script_domains.buildings.occupants = vec![if full {
                vec![handle; usize::from(u16::MAX)]
            } else {
                vec![]
            }];
            let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
                unreachable!()
            };
            soldier.soldier.rider = rider;
            let mut door = search_door(outside, inside, 110.0);
            door.door_type = door_type;
            door.active = active;
            door.locked_npc_villain = locked;
            let behind = Position {
                x: door.point_in.x,
                y: door.point_in.y,
                sector: Some(inside),
                level: 0,
            };
            engine.script_domains.interactables.doors.push(door);
            let center = Position {
                x: 100.0,
                y: 100.0,
                sector: Some(outside),
                level: 0,
            };
            engine.execute_ai_seek_area(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                center,
                0,
                SeekFlags::HOUSE | SeekFlags::LOCATION_FIRST,
                crate::position_interface::vector_to_sector_0_to_15_iso(10.0, 0.0) as u16,
            );
            let ai = engine.seek_enemy(owner);
            let expected = if accepted { behind } else { center };
            assert_eq!(ai.seek_center, expected, "{name}");
            assert_eq!(
                ai.personal_seek_point_1.as_ref().unwrap().position,
                expected,
                "{name}"
            );
            assert_eq!(ai.my_seek_points, vec![1111], "{name}");
            assert_eq!(ai.base.current_state, AiState::Seeking, "{name}");
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingSeekpointWatchingSidewards,
                "{name}"
            );
            assert!(ai.base.timer_is_running, "{name}");
            assert_eq!(ai.base.when_does_timer_ring, 103, "{name}");
            assert_eq!(
                ai.base.substate_at_last_timer_launch,
                Substate::SeekingSeekpointWatchingSidewards,
                "{name}"
            );
        }
    }

    #[test]
    fn checkpoint_search_stop_preserves_macro_before_seek_state_callback() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultLookingForCharly;
        ai.base.macro_in_progress = true;
        ai.base.macro_command_offset = 23;
        ai.base.number_of_remaining_macro_bytes = 0;
        let (mut engine, assets, owner) = search_fixture(ai, Default::default(), true);
        engine.execute_ai_seek_area(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            Position {
                x: 100.0,
                y: 100.0,
                sector: Some(test_sector()),
                level: 0,
            },
            0,
            SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
            8,
        );
        let ai = engine.seek_enemy(owner);
        assert!(ai.base.macro_in_progress);
        assert_eq!(ai.base.macro_command_offset, 23);
        assert_eq!(ai.base.number_of_remaining_macro_bytes, 0);
        assert_eq!(ai.base.current_state, AiState::Seeking);
    }

    use crate::ai::AiGlobalState;
    use crate::coordinates::MapPoint;
    use crate::position_interface::SectorHandle;

    fn test_sector() -> SectorHandle {
        SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap())
    }

    fn search_fixture(
        mut ai: EnemyAi,
        global: AiGlobalState,
        indoors: bool,
    ) -> (EngineInner, LevelAssets, EntityId) {
        let mut engine = EngineInner::new();
        engine.control.frame_counter = 500;
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(9);
        let mut sector = crate::engine::test_support::square_sector(
            1,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(3000.0, 3000.0),
        );
        if indoors {
            sector.sector_type |= crate::sector::SectorType::BUILDING;
        }
        engine.world.fast_grid_mut().add_sector(sector, 0);
        engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                2,
                8,
                MapPoint::new(0.0, 0.0),
                MapPoint::new(3000.0, 3000.0),
            ),
            8,
        );
        let mut actor = crate::engine::test_support::actors::make_test_ai_soldier(
            crate::element::Camp::Lacklandists,
        );
        actor
            .element_data_mut()
            .set_position_map(MapPoint::new(25.0, 25.0));
        actor.element_data_mut().set_sector(Some(test_sector()));
        ai.base.outbox = Default::default();
        let owner = engine.add_test_entity(actor);
        ai.base.me = owner.index();
        *engine
            .get_entity_mut(owner)
            .unwrap()
            .enemy_ai_mut()
            .unwrap() = ai;
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "search_test.scs",
        ));
        engine.ai.global.seek_points = global.seek_points;
        (engine, assets, owner)
    }
    #[test]
    fn seek_area_obligatory_selection_respects_original_finite_sentinel() {
        let sim = crate::sim_rng::test_context();
        let ai = EnemyAi::new(131);
        let center = Position {
            x: 1_585.620_1,
            y: 2_454.293_2,
            sector: Some(test_sector()),
            level: 0,
        };
        let global = AiGlobalState {
            seek_points: vec![
                SeekPoint {
                    position: Position {
                        x: 1547.0,
                        y: 2488.0,
                        sector: Some(test_sector()),
                        level: 0,
                    },
                    frame_when_full_interest: 0,
                    directions: vec![],
                    last_calculated_interest: 100,
                    locked: false,
                    id: 212,
                },
                SeekPoint {
                    position: Position {
                        x: 1753.0,
                        y: 2670.0,
                        sector: Some(test_sector()),
                        level: 0,
                    },
                    frame_when_full_interest: 0,
                    directions: vec![],
                    last_calculated_interest: 100,
                    locked: false,
                    id: 218,
                },
            ],
            ..Default::default()
        };

        let (mut engine, assets, owner) = search_fixture(ai, global, true);
        engine.execute_ai_seek_area(&sim, &assets, owner, center, 300, SeekFlags::empty(), 8);
        let ai = engine.seek_enemy(owner);

        assert_eq!(ai.my_seek_points.first(), Some(&212));
    }

    #[test]
    fn seek_next_point_preserves_the_search_center() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        let search_center = Position {
            x: 1_397.773,
            y: 1_864.478_5,
            sector: Some(test_sector()),
            level: 0,
        };
        let route_point = Position {
            x: 1236.0,
            y: 1589.0,
            sector: SectorHandle::new(2).map(|sector| {
                sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(1).unwrap())
            }),
            level: 8,
        };
        ai.base.seek_position = search_center;
        ai.my_seek_points.push(1111);
        ai.personal_seek_point_1 = Some(SeekPoint {
            position: route_point,
            frame_when_full_interest: 0,
            directions: vec![4, 10, 15],
            last_calculated_interest: 100,
            locked: false,
            id: 1111,
        });

        let (mut engine, assets, owner) = search_fixture(ai, Default::default(), false);
        engine.execute_ai_seek_next_point(&sim, &assets, owner);

        assert_eq!(
            engine.seek_enemy(owner).base.last_goto_destination,
            route_point
        );
        assert_eq!(engine.seek_enemy(owner).base.seek_position, search_center);
    }

    #[test]
    fn locked_seek_point_skips_interest_recalculation_and_acceptance_draw() {
        use crate::sim_rng::{RngSite, with_draw_trace};

        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        ai.my_seek_points = vec![0, 1];
        let locked_position = Position {
            x: 100.0,
            y: 100.0,
            sector: Some(test_sector()),
            ..Position::default()
        };
        let accepted_position = Position {
            x: 200.0,
            y: 100.0,
            sector: Some(test_sector()),
            ..Position::default()
        };
        let global = AiGlobalState {
            seek_points: vec![
                SeekPoint {
                    position: locked_position,
                    frame_when_full_interest: 1_000,
                    directions: vec![2],
                    last_calculated_interest: 7,
                    locked: true,
                    id: 0,
                },
                SeekPoint {
                    position: accepted_position,
                    frame_when_full_interest: 0,
                    directions: vec![4],
                    last_calculated_interest: 3,
                    locked: false,
                    id: 1,
                },
            ],
            ..Default::default()
        };
        let (mut engine, assets, owner) = search_fixture(ai, global, false);

        let (_, draws) = with_draw_trace(|| {
            engine.execute_ai_seek_next_point(&sim, &assets, owner);
        });

        assert_eq!(draws, [RngSite::SeekPointAcceptance]);
        assert_eq!(engine.ai.global.seek_points[0].last_calculated_interest, 7);
        assert!(!engine.ai.global.seek_points[0].locked);
        assert_eq!(
            engine.ai.global.seek_points[1].last_calculated_interest,
            100
        );
        assert!(engine.ai.global.seek_points[1].locked);
        assert_eq!(engine.seek_enemy(owner).actual_seek_point, Some(1));
        assert_eq!(
            engine.seek_enemy(owner).base.last_goto_destination,
            accepted_position
        );
    }

    #[test]
    fn unlocked_seek_point_recalculates_draws_subtracts_and_locks() {
        use crate::sim_rng::{RngSite, with_draw_trace};

        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        ai.my_seek_points.push(0);
        let destination = Position {
            x: 300.0,
            y: 100.0,
            sector: Some(test_sector()),
            ..Position::default()
        };
        let mut global = AiGlobalState::default();
        global.seek_points.push(SeekPoint {
            position: destination,
            frame_when_full_interest: 501,
            directions: vec![6],
            last_calculated_interest: 7,
            locked: false,
            id: 0,
        });
        let (mut engine, assets, owner) = search_fixture(ai, global, false);

        let (_, draws) = with_draw_trace(|| {
            engine.execute_ai_seek_next_point(&sim, &assets, owner);
        });

        assert_eq!(draws, [RngSite::SeekPointAcceptance]);
        assert_eq!(
            engine.ai.global.seek_points[0].last_calculated_interest,
            100
        );
        assert_eq!(
            engine.ai.global.seek_points[0].frame_when_full_interest,
            5_501
        );
        assert!(engine.ai.global.seek_points[0].locked);
        assert_eq!(engine.seek_enemy(owner).actual_seek_point, Some(0));
        assert_eq!(
            engine.seek_enemy(owner).base.last_goto_destination,
            destination
        );
    }

    #[test]
    fn beggar_detour_retains_old_seek_point_for_second_unlock() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(223);
        ai.actual_seek_point = Some(0);

        ai.positions_of_beggars_to_control.push(Position {
            x: 50.0,
            y: 60.0,
            sector: Some(test_sector()),
            ..Position::default()
        });

        let next_position = Position {
            x: 300.0,
            y: 400.0,
            sector: Some(test_sector()),
            ..Position::default()
        };
        let global = AiGlobalState {
            seek_points: vec![
                SeekPoint {
                    position: Position {
                        x: 1176.0,
                        y: 1958.0,
                        sector: Some(test_sector()),
                        ..Position::default()
                    },
                    frame_when_full_interest: 0,
                    directions: vec![2],
                    last_calculated_interest: 55,
                    locked: true,
                    id: 0,
                },
                SeekPoint {
                    position: next_position,
                    frame_when_full_interest: 0,
                    directions: vec![4],
                    last_calculated_interest: 100,
                    locked: false,
                    id: 1,
                },
            ],
            ..Default::default()
        };

        let (mut engine, assets, owner) = search_fixture(ai, global, false);
        let beggar =
            engine.add_test_entity(crate::engine::test_support::actors::make_test_civilian(
                crate::element::Posture::Leisure,
            ));
        engine
            .seek_enemy_mut(owner)
            .beggars_to_control
            .push(beggar.index());
        engine.execute_ai_seek_next_point(&sim, &assets, owner);

        assert_eq!(engine.seek_enemy(owner).actual_seek_point, Some(0));
        assert!(!engine.ai.global.seek_points[0].locked);
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::SeekingSeekpointApproachingBeggar
        );

        // A different investigator selects the shared point while this AI is
        // away identifying the beggar. Retaining the point identity makes the
        // resumed next-point selection clear that intervening lock again.
        engine.ai.global.seek_points[0].locked = true;
        engine.seek_enemy_mut(owner).beggar_to_examine = None;
        engine.seek_enemy_mut(owner).my_seek_points.push(1);

        engine.execute_ai_seek_next_point(&sim, &assets, owner);

        assert!(!engine.ai.global.seek_points[0].locked);
        assert_eq!(engine.seek_enemy(owner).actual_seek_point, Some(1));
        assert!(engine.ai.global.seek_points[1].locked);
        assert_eq!(
            engine.seek_enemy(owner).base.last_goto_destination,
            next_position
        );
    }
}
