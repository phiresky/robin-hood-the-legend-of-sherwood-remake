//! Civilian reporting calls execute with actor borrows confined to statements.

use super::*;
use crate::ai::{AiState, GotoFlags, Remark, Stimulus, StimulusInfo, Substate};
use crate::parameters_ai::{AI_STANDARD_PANIC_RUNS, AI_TALK_DISTANCE};

impl EngineInner {
    pub(in crate::engine) fn execute_friendly_callback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
        ctx: &AiContext,
    ) -> Option<bool> {
        let substate = self
            .world
            .entities
            .get(owner)?
            .friendly_ai()?
            .base
            .current_substate;
        let event = stimulus.stimulus_type;
        if event == StimulusType::EventSeesSoldier
            && substate == Substate::SeekingCivilianRunningToSoldier
        {
            let StimulusInfo::Human(target) = stimulus.info else {
                panic!("civilian soldier sighting requires a human target");
            };
            self.reporting_civilian_mut(owner).base.antagonist = Some(target);
            self.clear_reporting_friends(sim, assets, owner);
            self.civilian_call_alert(sim, assets, owner, ctx, false);
            return Some(false);
        }
        if !matches!(
            event,
            StimulusType::EventReachPoint
                | StimulusType::EventDone
                | StimulusType::EventTimer
                | StimulusType::CallYourTalk1
                | StimulusType::CallYourTalk2
                | StimulusType::CallYourTalk3
                | StimulusType::EventMyTalk1
                | StimulusType::EventMyTalk2
                | StimulusType::EventMyTalk3
        ) {
            return None;
        }
        match substate {
            Substate::SeekingCivilianRunningToSoldier => {
                if event == StimulusType::EventReachPoint {
                    let target = self.reporting_target(owner);
                    let state = self
                        .world
                        .entities
                        .expect_ai_controller(target, format_args!("civilian alert target"))
                        .current_state;
                    if state == AiState::Default {
                        let position = self.live_ai_position(target);
                        let own_position = self.live_ai_position(owner);
                        let dx = position.x - own_position.x;
                        let dy = position.y - own_position.y;
                        if dx * dx + dy * dy > (AI_TALK_DISTANCE as f32).powi(2) {
                            self.approach_reporting_soldier(sim, assets, owner);
                        } else {
                            self.civilian_call_alert(sim, assets, owner, ctx, true);
                        }
                    } else {
                        let grid = &self.world.fast_grid;
                        let doors = self.script_domains.interactables.doors.as_slice();
                        let civilian = self
                            .world
                            .entities
                            .get_mut(owner)
                            .and_then(Entity::friendly_ai_mut)
                            .expect("civilian alert caller");
                        if !civilian.alert_soldier(
                            sim,
                            civilian.base.seek_position,
                            0,
                            crate::ai_friendly::AlertSoldierFailureContinuation::ReturnToDuty,
                            ctx,
                            Some(grid),
                            Some(doors),
                        ) {
                            self.execute_ai_return_to_duty(
                                sim,
                                assets,
                                owner,
                                crate::ai::DutyFlags::empty(),
                            );
                        }
                    }
                }
            }
            Substate::SeekingCivilianRunningToSoldierSeen => {
                if matches!(
                    event,
                    StimulusType::EventReachPoint | StimulusType::EventTimer
                ) {
                    let target = self.reporting_target(owner);
                    let waiting = self
                        .world
                        .entities
                        .expect_ai_controller(target, format_args!("civilian waiting target"))
                        .current_substate
                        == Substate::SeekingWaitForAlertingCivilian;
                    if !waiting {
                        self.execute_ai_return_to_duty(
                            sim,
                            assets,
                            owner,
                            crate::ai::DutyFlags::empty(),
                        );
                    } else if event == StimulusType::EventTimer {
                        self.reporting_civilian_mut(owner)
                            .base
                            .launch_timer(20, ctx.frame);
                    } else {
                        self.reporting_state(
                            sim,
                            assets,
                            owner,
                            Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                        );
                        self.reporting_civilian_mut(owner)
                            .base
                            .launch_timer(10, ctx.frame);
                    }
                }
            }
            Substate::SeekingCivilianGiveAlertingReportToSoldierStart => {
                if event == StimulusType::EventTimer {
                    self.reporting_state(
                        sim,
                        assets,
                        owner,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
                    );
                    let target = self.reporting_target(owner);
                    self.execute_ai_callback(
                        sim,
                        assets,
                        target,
                        &Stimulus::with_human(StimulusType::CallReport, owner.index()),
                    );
                    self.reporting_civilian_mut(owner)
                        .base
                        .say(Remark::CivDenunciates);
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                    let position = self.reporting_civilian_mut(owner).base.seek_position;
                    self.duty_point_to(sim, assets, owner, position);
                }
            }
            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint => {
                if event == StimulusType::EventDone {
                    self.reporting_state(
                        sim,
                        assets,
                        owner,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
                    );
                    let target = self.reporting_target(owner);
                    let position = self.live_ai_position(target);
                    let elevation = self
                        .expect_entity(target, "civilian report facing target")
                        .element_data()
                        .position()
                        .z as i16;
                    self.duty_face_position_at_elevation(
                        sim,
                        assets,
                        owner,
                        position,
                        f32::from(elevation),
                    );
                    let frame = self.control.frame_counter;
                    self.reporting_civilian_mut(owner)
                        .base
                        .launch_timer(30, frame);
                }
            }
            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd => {
                if event == StimulusType::EventTimer {
                    let civilian = self.reporting_civilian_mut(owner);
                    civilian.panic_from_point_at(
                        civilian.base.seek_position,
                        AI_STANDARD_PANIC_RUNS as u8,
                    );
                }
            }
            _ => return None,
        }
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        Some(false)
    }

    fn reporting_civilian_mut(&mut self, owner: EntityId) -> &mut crate::ai_friendly::FriendlyAi {
        self.world
            .entities
            .get_mut(owner)
            .and_then(Entity::friendly_ai_mut)
            .expect("reporting civilian lost its brain")
    }

    fn reporting_target(&self, owner: EntityId) -> EntityId {
        let handle = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("reporting civilian"))
            .antagonist
            .expect("reporting civilian requires an antagonist")
            .get();
        self.expect_human_id_for_ai_handle(handle, "reporting civilian antagonist")
    }

    fn reporting_state(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        substate: Substate,
    ) {
        self.reporting_civilian_mut(owner)
            .set_state(AiState::Seeking, substate);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn clear_reporting_friends(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.reporting_civilian_mut(owner)
            .base
            .outbox
            .actor
            .delete_detectable_type(crate::element::DetectableType::Friend);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn civilian_call_alert(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        ctx: &AiContext,
        reached: bool,
    ) {
        let target = self.reporting_target(owner);
        let accepted = self.execute_ai_callback(
            sim,
            assets,
            target,
            &Stimulus::with_human(StimulusType::CallAlert, owner.index()),
        );
        if !accepted {
            self.reporting_civilian_mut(owner)
                .panic_undirected(AI_STANDARD_PANIC_RUNS as u8);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            return;
        }
        if reached {
            self.clear_reporting_friends(sim, assets, owner);
        }
        self.reporting_state(
            sim,
            assets,
            owner,
            Substate::SeekingCivilianRunningToSoldierSeen,
        );
        if reached {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            );
        } else {
            self.reporting_civilian_mut(owner)
                .base
                .say(Remark::CivCallsSoldier);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.approach_reporting_soldier(sim, assets, owner);
            self.reporting_civilian_mut(owner)
                .base
                .launch_timer(20, ctx.frame);
        }
    }

    fn approach_reporting_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.reporting_target(owner);
        let entity = self.expect_entity(target, "civilian approach forecast");
        let passing_door =
            selected_pass_door_movement(&self.orders.sequence_manager, target).is_some();
        let input = extract_exact_forecast_input(self, entity, passing_door)
            .expect("soldier forecast requires an actor");
        let position = crate::ai::forecast_destination_for_ia(
            sim,
            &input,
            &self.script_domains.interactables.doors,
            &self.world.fast_grid.level.sectors,
            &self.world.fast_grid.level.sector_number_map,
        )
        .position;
        self.duty_go_near(
            sim,
            assets,
            owner,
            position,
            AI_TALK_DISTANCE,
            GotoFlags::RUN,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_civilian};

    fn reporting_pair(substate: Substate) -> (EngineInner, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        let mut entity = make_test_civilian(crate::element::Posture::Upright);
        let Entity::Civilian(civilian) = &mut entity else {
            unreachable!()
        };
        civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::default());
        let owner = engine.add_test_entity(entity);
        let soldier = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = substate;
        ai.base.antagonist = Some(crate::ai::AiEntityHandle::new(soldier.index()));
        (engine, owner, soldier)
    }

    #[test]
    #[should_panic(expected = "reporting civilian antagonist: missing entity")]
    fn running_to_soldier_requires_live_antagonist() {
        let (mut engine, owner, _) = reporting_pair(Substate::SeekingCivilianRunningToSoldier);
        engine.reporting_civilian_mut(owner).base.antagonist =
            Some(crate::ai::AiEntityHandle::new(42));
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
            &Stimulus::new(StimulusType::EventReachPoint),
            &AiContext::test_fixture(),
        );
    }

    #[test]
    fn waiting_for_report_reads_target_brain_without_entity_views() {
        let (mut engine, owner, soldier) =
            reporting_pair(Substate::SeekingCivilianRunningToSoldierSeen);
        engine
            .world
            .entities
            .expect_ai_controller_mut(soldier, format_args!("test report listener"))
            .current_substate = Substate::SeekingWaitForAlertingCivilian;
        let ctx = AiContext::test_fixture();
        assert!(ctx.entity_view(soldier.index()).is_none());
        assert_eq!(
            engine.execute_friendly_callback(
                &crate::sim_rng::test_context(),
                &LevelAssets::default(),
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
                &ctx
            ),
            Some(false)
        );
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingCivilianGiveAlertingReportToSoldierStart
        );
        assert!(ai.base.timer_is_running);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    }

    #[test]
    fn reporting_family_leaves_interrupts_to_the_general_dispatcher() {
        let (mut engine, owner, _) = reporting_pair(Substate::SeekingCivilianRunningToSoldierSeen);
        for event in [
            StimulusType::EventReturnToDuty,
            StimulusType::EventLoseConsciousness,
            StimulusType::EventView,
            StimulusType::EventCouldntReachPoint,
        ] {
            assert_eq!(
                engine.execute_friendly_callback(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::default(),
                    owner,
                    &Stimulus::new(event),
                    &AiContext::test_fixture()
                ),
                None
            );
        }
    }

    #[test]
    fn report_point_done_faces_live_target_then_launches_timer() {
        let (mut engine, owner, _) =
            reporting_pair(Substate::SeekingCivilianGiveAlertingReportToSoldierPoint);
        let mut ctx = AiContext::test_fixture();
        ctx.position = engine.live_ai_position(owner);
        ctx.direction = crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 0.0) as u16;
        ctx.self_action_state = crate::element::ActionState::Waiting;
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_direction_instantly(ctx.direction as i16);
        entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
            &Stimulus::new(StimulusType::EventDone),
            &ctx,
        );
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd
        );
        assert!(ai.base.timer_is_running);
        assert!(ai.base.already_turned);
    }
}
