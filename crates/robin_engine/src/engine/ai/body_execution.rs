//! Body examination and its result-dependent search decisions.
use super::*;
use crate::ai::{
    AiState, BodyReaction, DutyFlags, EmoticonType, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::parameters_ai;
use crate::profiles::ProfileRank;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_body_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        operation: BodyReaction,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        match operation {
            BodyReaction::Seen { body } => self.execute_ai_seen_body(sim, assets, owner, body),
            BodyReaction::ReactionTimer => self.execute_ai_body_reaction_timer(sim, assets, owner),
            BodyReaction::Examine { body } => {
                let body = self.expect_human_id_for_ai_handle(body, "body examination target");
                self.execute_seek_body(sim, assets, owner, body);
            }
            BodyReaction::BodyTimer => {
                let body = self.body_target(owner);
                let entity = self.expect_entity(body, "body timer target");
                if entity.human_life_points() > 0
                    && !entity.is_unconscious()
                    && self.npc_is_detecting_human(assets, owner, body, self.control.frame_counter)
                {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                } else {
                    self.body_timer(owner, 10);
                }
            }
            BodyReaction::Arrival => self.execute_body_arrival(sim, assets, owner),
            BodyReaction::DeadBodyTimer => {
                if self
                    .expect_entity(owner, "body observer")
                    .soldier_data()
                    .is_some_and(|s| s.rider)
                {
                    let center = self.live_ai_position(owner);
                    self.body_seek(sim, assets, owner, center, SeekFlags::BODY_SEEK);
                } else {
                    if !self.seek_enemy(owner).seen_dead_body {
                        let ai = self.seek_enemy_mut(owner);
                        ai.seen_dead_body = true;
                        self.execute_ai_speech(
                            sim,
                            assets,
                            owner,
                            crate::ai::AiSpeechAttempt {
                                remark: Remark::BahIlBougePus,
                                flags: 0,
                            },
                        );
                    }
                    if self.execute_seek_other_bodies(sim, assets, owner) {
                        let center = self.live_ai_position(owner);
                        self.seek_enemy_mut(owner)
                            .base
                            .my_reconnaissance_report
                            .update(ReportType::DeadBody, center);
                    } else {
                        let center = self.live_ai_position(owner);
                        self.execute_dead_body_alert(sim, assets, owner, center);
                    }
                }
            }
            BodyReaction::SleeperTimer => {
                if !self.execute_seek_other_bodies(sim, assets, owner) {
                    let report = &self.seek_enemy(owner).base.my_reconnaissance_report;
                    if report.report_type == ReportType::DeadBody {
                        let center = report.seek_position;
                        self.execute_dead_body_alert(sim, assets, owner, center);
                    } else {
                        self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                    }
                }
            }
            BodyReaction::NetDone => {
                let body = self.body_target(owner);
                let entity = self.expect_entity(body, "uncovered body");
                let stuck = entity
                    .human_data()
                    .expect("uncovered body must be human")
                    .stuck_under_nets_counter
                    > 0;
                if stuck || entity.human_life_points() <= 0 || entity.is_unconscious() {
                    self.execute_seek_body(sim, assets, owner, body);
                } else {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            BodyReaction::Unreachable => {
                if !self.execute_seek_other_bodies(sim, assets, owner) {
                    let center = self.live_ai_position(owner);
                    self.body_seek(sim, assets, owner, center, SeekFlags::empty());
                }
            }
            BodyReaction::DeadBodyAlert { center } => {
                self.execute_dead_body_alert(sim, assets, owner, center)
            }
        }
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn body_target(&self, owner: EntityId) -> EntityId {
        let body = self
            .seek_enemy(owner)
            .base
            .detected_body
            .expect("body operation requires detected body");
        self.expect_human_id_for_ai_handle(body.get(), "detected body")
    }

    fn body_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner)
            .base
            .launch_timer(duration, frame);
    }

    fn body_seek(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
        flags: SeekFlags,
    ) {
        self.execute_ai_seek_area(
            sim,
            assets,
            owner,
            center,
            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
            flags,
            UNDEFINED_DIRECTION,
        );
    }

    fn execute_body_arrival(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.seek_enemy_mut(owner)
            .base
            .set_emoticon(EmoticonType::XMark);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let body = self.body_target(owner);
        let delta = self
            .expect_entity(body, "body arrival target")
            .element_data()
            .position()
            - self
                .expect_entity(owner, "body arrival owner")
                .element_data()
                .position();
        let disappeared = delta.x.abs().max(delta.y.abs()).max(delta.z.abs())
            > (2 * parameters_ai::AI_STOP_BEFORE_BODY_STEPS) as f32;
        if disappeared {
            if !self.execute_seek_other_bodies(sim, assets, owner) {
                let body = self.body_target(owner);
                let entity = self.expect_entity(body, "missing body");
                if entity.is_unconscious() || entity.human_life_points() <= 0 {
                    self.seek_enemy_mut(owner)
                        .base
                        .outbox
                        .actor
                        .add_detectable((body, crate::element::DetectableType::Body));
                }
                self.seek_enemy_mut(owner)
                    .base
                    .set_emoticon(EmoticonType::QuestionMark);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                match self.seek_enemy(owner).get_rank() {
                    ProfileRank::Officer => {
                        let center = self.live_ai_position(owner);
                        if self.execute_ai_alert_soldiers(sim, assets, owner, center, 0) {
                            return;
                        }
                    }
                    ProfileRank::Soldier | ProfileRank::Knight => {}
                    ProfileRank::None => return,
                }
                let flags = SeekFlags::LOCATION_FIRST | self.seek_enemy(owner).seek_flags;
                let center = self.live_ai_position(owner);
                self.body_seek(sim, assets, owner, center, flags);
            }
            return;
        }
        let rider = self
            .expect_entity(owner, "body arrival owner")
            .soldier_data()
            .is_some_and(|s| s.rider);
        let body = self.body_target(owner);
        let entity = self.expect_entity(body, "body classification");
        if rider || entity.human_life_points() <= 0 {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingBodyLookingDeadBody,
            );
            let body = self.body_target(owner);
            self.seek_enemy_mut(owner)
                .already_seen_bodies
                .push(body.index());
            self.body_timer(owner, 50);
            self.seek_enemy_mut(owner)
                .base
                .set_emoticon(EmoticonType::XMark);
        } else if entity.element_data().posture() == crate::element::Posture::Tied
            || entity
                .ai_controller()
                .is_some_and(|ai| ai.current_substate == Substate::SleepingUnconscious)
        {
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: Remark::AwakensSleeperr,
                    flags: 0,
                },
            );
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingBodyAwakeningSleeperr,
            );
            self.stop_ai_owner(sim, assets, owner);
            let body = self.body_target(owner);
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::WakeUp,
                Some(owner),
                Some(body),
            ));
            self.seek_enemy_mut(owner)
                .base
                .outbox
                .actor
                .launch_sequences
                .push(sequence);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.body_timer(owner, 50);
            self.seek_enemy_mut(owner).base.clear_emoticon();
        } else {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
        }
    }

    fn execute_dead_body_alert(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
    ) {
        self.seek_enemy_mut(owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::DeadBody, center);
        match self.seek_enemy(owner).get_rank() {
            ProfileRank::Soldier => {
                let entity = self.expect_entity(owner, "body alert owner");
                let ai = self.seek_enemy(owner);
                let seek_first = ai.base.blood_alcohol as i32
                    <= parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                    && entity.is_active()
                    && !self.entity_data_in_building_sector(entity.element_data())
                    && ai.soldier_profile_initiative >= 50
                    && ai.base.antagonist.is_none();
                let flags = if seek_first {
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK | SeekFlags::LOOK_FOR_HELP_AFTER
                } else {
                    if self.execute_ai_alert_officer(sim, assets, owner) {
                        return;
                    }
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
                };
                let radius = self.body_alert_radius(owner);
                self.execute_ai_seek_area(
                    sim,
                    assets,
                    owner,
                    center,
                    radius,
                    flags,
                    UNDEFINED_DIRECTION,
                );
            }
            ProfileRank::Officer => {
                let direction = self
                    .expect_entity(owner, "body alert facing")
                    .element_data()
                    .direction() as u16
                    ^ 8;
                self.duty_face_direction(sim, assets, owner, direction);
                let position = self.live_ai_position(owner);
                if !self.execute_ai_alert_soldiers(
                    sim,
                    assets,
                    owner,
                    position,
                    SeekFlags::BODY_SEEK.bits(),
                ) {
                    let radius = self.body_alert_radius(owner);
                    self.execute_ai_seek_area(
                        sim,
                        assets,
                        owner,
                        center,
                        radius,
                        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                        UNDEFINED_DIRECTION,
                    );
                }
            }
            ProfileRank::Knight => {
                let position = self.live_ai_position(owner);
                let radius = self.body_alert_radius(owner);
                self.execute_ai_seek_area(
                    sim,
                    assets,
                    owner,
                    position,
                    radius,
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                    UNDEFINED_DIRECTION,
                );
            }
            ProfileRank::None => {}
        }
    }

    fn body_alert_radius(&self, owner: EntityId) -> u16 {
        if self.seek_enemy(owner).soldier_profile_duty {
            parameters_ai::AI_SOD_DEAD_BODY_SEEK_RADIUS as u16
        } else {
            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiEntityHandle, SeekPoint};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::{
        ElementData, ElementKind, ElementNet, NetData, ObjectData, Posture, ProjectileData,
    };
    use crate::engine::test_support::{
        actors::{make_test_ai_soldier, make_test_civilian},
        square_sector,
    };

    fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.control.frame_counter = 100;
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(2000.0, 2000.0)),
            0,
        );
        let sector = crate::ai::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let owner =
            engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let body = engine.add_test_entity(make_test_civilian(Posture::Leisure));
        for (id, x) in [(owner, 100.0), (body, 400.0)] {
            let entity = engine.get_entity_mut(id).unwrap();
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, 100.0, 0.0));
            entity.element_data_mut().set_sector(Some(sector));
            entity.element_data_mut().active = true;
            entity.npc_data_mut().unwrap().life_points = 50;
            entity
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-10.0, -5.0),
                    crate::coordinates::MapVec::new(10.0, 5.0),
                ));
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "body_execution_test.scs",
        ));
        let initial = engine.live_ai_position(owner);
        let ai = engine.seek_enemy_mut(owner);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.initial_position = initial;
        ai.base.special_action = true;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingTakingNet;
        ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
        engine.enter_ai_think_frame(owner);
        (engine, assets, owner, body)
    }

    fn covering_net(
        engine: &mut EngineInner,
        victim: EntityId,
        x: f32,
        active: bool,
        crumpled: bool,
    ) -> EntityId {
        let mut element = ElementData::default();
        element.kind = ElementKind::ObjectNet;
        element.active = active;
        element.set_position(WorldPoint3D::new(x, 100.0, 0.0));
        element.set_sector(engine.get_entity(victim).unwrap().element_data().sector());
        engine.add_test_entity(Entity::Net(ElementNet {
            element,
            object: ObjectData {
                object_type: crate::element::ObjectType::Net,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
            net: NetData {
                victims: vec![victim],
                crumpled,
                ..Default::default()
            },
        }))
    }

    #[test]
    fn examine_body_selects_live_covering_net_and_current_radius() {
        for crumpled in [false, true] {
            let (mut engine, assets, owner, body) = fixture();
            engine
                .get_entity_mut(body)
                .unwrap()
                .human_data_mut()
                .unwrap()
                .stuck_under_nets_counter = 1;
            covering_net(&mut engine, body, 125.0, false, false);
            let farther = covering_net(&mut engine, body, 450.0, true, false);
            let chosen = covering_net(&mut engine, body, 350.0, true, crumpled);
            engine.execute_ai_body_reaction(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                BodyReaction::Examine { body: body.index() },
            );
            let ai = engine.seek_enemy(owner);
            assert_eq!(
                ai.base.detected_body,
                Some(AiEntityHandle::new(body.index()))
            );
            assert_eq!(
                ai.base.interesting_object,
                Some(AiEntityHandle::new(chosen.index()))
            );
            assert_ne!(
                ai.base.interesting_object,
                Some(AiEntityHandle::new(farther.index()))
            );
            assert_eq!(ai.base.current_state, AiState::Seeking);
            assert_eq!(ai.base.current_substate, Substate::SeekingNet);
            assert_eq!(
                ai.base.last_goto_destination,
                engine.live_ai_position(chosen)
            );
            assert_eq!(
                ai.base.stop_before_end_of_path_distance,
                if crumpled { 25 } else { 55 }
            );
            assert_eq!(ai.base.when_does_timer_ring, 110);
        }
    }

    #[test]
    #[should_panic(expected = "body examination target")]
    fn examining_missing_body_rejects_invalid_identity() {
        let (mut engine, assets, owner, _) = fixture();
        engine.execute_ai_body_reaction(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            BodyReaction::Examine { body: u32::MAX - 1 },
        );
    }

    #[test]
    fn body_queue_prunes_awake_civilian_and_examines_the_next_unconscious_human() {
        let (mut engine, mut assets, owner, recovered) = fixture();
        let mut down = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        down.element_data_mut()
            .set_position(WorldPoint3D::new(658.0, 100.0, 0.0));
        down.element_data_mut()
            .set_sector(engine.get_entity(owner).unwrap().element_data().sector());
        down.human_data_mut().unwrap().unconscious = true;
        let down = engine.add_test_entity(down);
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.seek_enemy_mut(owner).other_bodies_to_examine =
            vec![recovered.index(), down.index()];
        engine.execute_ai_body_reaction(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            BodyReaction::SleeperTimer,
        );
        let ai = engine.seek_enemy(owner);
        assert!(ai.other_bodies_to_examine.is_empty());
        assert_eq!(
            ai.base.detected_body,
            Some(AiEntityHandle::new(down.index()))
        );
        assert_eq!(ai.base.current_substate, Substate::SeekingBody);
        assert_eq!(ai.base.seek_position, engine.live_ai_position(down));
        assert_eq!(ai.base.last_goto_destination, engine.live_ai_position(down));
    }

    #[test]
    fn net_completion_distinguishes_awake_civilians_from_dead_or_unconscious_bodies() {
        for (dead, unconscious) in [(false, false), (true, false), (false, true)] {
            let (mut engine, assets, owner, body) = fixture();
            let entity = engine.get_entity_mut(body).unwrap();
            entity.human_data_mut().unwrap().unconscious = unconscious;
            entity.npc_data_mut().unwrap().life_points = if dead { 0 } else { 50 };
            engine.execute_ai_body_reaction(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                BodyReaction::NetDone,
            );
            let ai = engine.seek_enemy(owner);
            if dead || unconscious {
                assert_eq!(ai.base.current_substate, Substate::SeekingBody);
                assert_eq!(ai.base.last_goto_destination, engine.live_ai_position(body));
                assert_eq!(ai.base.when_does_timer_ring, 110);
            } else {
                assert_eq!(
                    ai.base.current_state,
                    AiState::Default,
                    "an awake civilian returns the rescuer to duty despite never being combat-ready"
                );
                assert_ne!(ai.base.current_substate, Substate::SeekingBody);
            }
            assert_eq!(engine.ai_think_depth(), 1);
        }
    }

    #[test]
    fn net_completion_rechecks_remaining_covering_nets_before_body_classification() {
        let (mut engine, assets, owner, body) = fixture();
        engine
            .get_entity_mut(body)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .stuck_under_nets_counter = 1;
        let net = covering_net(&mut engine, body, 350.0, true, false);
        engine.execute_ai_body_reaction(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            BodyReaction::NetDone,
        );
        let ai = engine.seek_enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::SeekingNet);
        assert_eq!(
            ai.base.interesting_object,
            Some(AiEntityHandle::new(net.index()))
        );
    }

    #[test]
    fn dead_body_alert_uses_live_soldier_duty_radius_after_failed_officer_alert() {
        for duty in [false, true] {
            let (mut engine, mut assets, owner, _) = fixture();
            engine.world.fast_grid_mut().level_mut().sectors[0].sector_type |=
                crate::sector::SectorType::BUILDING;
            let center = engine.live_ai_position(owner);
            for id in 0..3 {
                engine.ai.global.seek_points.push(SeekPoint {
                    id,
                    position: Position {
                        x: center.x + 200.0 + f32::from(id) * 20.0,
                        ..center
                    },
                    frame_when_full_interest: 0,
                    directions: vec![4],
                    last_calculated_interest: 100,
                    locked: false,
                });
            }
            engine.seek_enemy_mut(owner).soldier_profile_duty = duty;
            let profile = engine
                .get_entity(owner)
                .unwrap()
                .soldier_data()
                .unwrap()
                .soldier_profile_index;
            std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[usize::from(profile)]
                .duty = duty;
            engine.execute_ai_body_reaction(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                BodyReaction::DeadBodyAlert { center },
            );
            let ai = engine.seek_enemy(owner);
            assert_eq!(
                ai.base.my_reconnaissance_report.report_type,
                ReportType::DeadBody
            );
            assert_eq!(ai.base.my_reconnaissance_report.seek_position, center);
            assert_eq!(
                ai.seek_flags,
                SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
            );
            assert_eq!(ai.personal_seek_point_2.as_ref().unwrap().position, center);
            assert_eq!(
                ai.my_seek_points.iter().filter(|id| **id < 3).count(),
                if duty { 1 } else { 3 },
                "standard radius sets the expected point count, not a hard membership boundary"
            );
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingSeekpointWatchingSidewards
            );
        }
    }
}
