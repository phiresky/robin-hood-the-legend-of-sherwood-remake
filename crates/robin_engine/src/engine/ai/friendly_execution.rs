//! Civilian decisions query live actors and complete callbacks between short borrows.

use super::*;
use crate::ai::{AiState, GotoFlags, Remark, Stimulus, StimulusInfo, Substate};
use crate::element::Human as _;
use crate::parameters_ai::{AI_STANDARD_PANIC_RUNS, AI_TALK_DISTANCE};

impl EngineInner {
    pub(in crate::engine) fn execute_friendly_callback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let frame = self.control.frame_counter;
        let substate = self
            .world
            .entities
            .get(owner)?
            .friendly_ai()?
            .base
            .current_substate;
        let event = stimulus.stimulus_type;
        if event == StimulusType::CallPatrolCoordinate {
            self.execute_ai_coordinate_patrol(sim, assets, owner, &stimulus.info);
            return Some(false);
        }
        if event == StimulusType::EventHear {
            let state = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("civilian hearing"))
                .current_state;
            if matches!(
                state,
                AiState::Sleeping | AiState::Default | AiState::Wondering | AiState::Seeking
            ) {
                if let StimulusInfo::Noise(noise) = stimulus.info {
                    self.civilian_hear(sim, assets, owner, &noise);
                }
            }
            return Some(false);
        }
        if event == StimulusType::EventSeesSoldier
            && substate == Substate::SeekingCivilianRunningToSoldier
        {
            let StimulusInfo::Human(target) = stimulus.info else {
                panic!("civilian soldier sighting requires a human target");
            };
            self.reporting_civilian_mut(owner).base.antagonist = Some(target);
            self.clear_reporting_friends(owner);
            self.civilian_call_alert(sim, assets, owner, false);
            return Some(false);
        }
        if let Some(result) = self.execute_friendly_behavior(sim, assets, owner, stimulus) {
            return Some(result);
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
            Substate::DefaultPatrolEnrouteWaiting => {
                if event == StimulusType::EventTimer {
                    let chief = self
                        .world
                        .entities
                        .expect_ai_controller(owner, format_args!("waiting civilian"))
                        .patrol_chief
                        .expect("waiting civilian requires a patrol chief");
                    let state = self
                        .world
                        .entities
                        .expect_ai_controller(chief, format_args!("civilian patrol chief"))
                        .current_state;
                    if matches!(state, AiState::Default | AiState::Wondering) {
                        let frame = self.control.frame_counter;
                        self.reporting_civilian_mut(owner)
                            .base
                            .launch_timer(200, frame);
                    } else {
                        self.execute_ai_return_to_duty(
                            sim,
                            assets,
                            owner,
                            crate::ai::DutyFlags::empty(),
                        );
                    }
                }
            }
            Substate::WonderingCivilianEnemyReactiontime
            | Substate::WonderingCivilianBodyReactiontime => {
                if event == StimulusType::EventTimer
                    && !self.civilian_alert_soldier(sim, assets, owner, false)
                {
                    self.execute_ai_speech(
                        sim,
                        assets,
                        owner,
                        crate::ai::AiSpeechAttempt {
                            remark: Remark::CivPanic,
                            flags: 0,
                        },
                    );

                    let center = self.reporting_civilian_mut(owner).base.seek_position;
                    self.execute_ai_panic(
                        sim,
                        assets,
                        owner,
                        Some(center),
                        AI_STANDARD_PANIC_RUNS as u8,
                        crate::ai::AlertLevel::Red,
                    );
                }
            }
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
                            self.civilian_call_alert(sim, assets, owner, true);
                        }
                    } else {
                        if !self.civilian_alert_soldier(sim, assets, owner, false) {
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
                            .launch_timer(20, frame);
                    } else {
                        self.reporting_state(
                            sim,
                            assets,
                            owner,
                            Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                        );
                        self.reporting_civilian_mut(owner)
                            .base
                            .launch_timer(10, frame);
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
                    self.execute_ai_speech(
                        sim,
                        assets,
                        owner,
                        crate::ai::AiSpeechAttempt {
                            remark: Remark::CivDenunciates,
                            flags: 0,
                        },
                    );

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
                    let center = civilian.base.seek_position;
                    self.execute_ai_panic(
                        sim,
                        assets,
                        owner,
                        Some(center),
                        AI_STANDARD_PANIC_RUNS as u8,
                        crate::ai::AlertLevel::Red,
                    );
                }
            }
            _ => return None,
        }

        Some(false)
    }

    fn civilian_hear(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        noise: &crate::ai::Noise,
    ) {
        match noise.noise_type {
            crate::ai::NoiseType::Pfiiit => {
                let Entity::Civilian(civilian) = self.expect_entity(owner, "civilian whistle")
                else {
                    unreachable!("friendly owner is not civilian")
                };
                if civilian.civilian.cached_civilian_type != crate::profiles::CivilianType::Child {
                    return;
                }
                self.reporting_civilian_mut(owner)
                    .base
                    .set_emoticon(crate::ai::EmoticonType::QuestionMark);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingWatchingWhistling,
                );
                let origin = noise
                    .origin
                    .position()
                    .expect("delivered whistle has no spatial layer");
                self.reporting_civilian_mut(owner).base.seek_position = origin;
                self.duty_face_position_at_elevation(
                    sim,
                    assets,
                    owner,
                    origin,
                    f32::from(noise.elevation),
                );
                let frame = self.control.frame_counter;
                self.reporting_civilian_mut(owner)
                    .base
                    .launch_timer(70, frame);
            }
            crate::ai::NoiseType::Aaargh => {
                let origin = noise
                    .origin
                    .position()
                    .expect("delivered scream has no spatial layer");
                self.reporting_civilian_mut(owner).base.seek_position = origin;
                if self.expect_entity(owner, "screaming civilian").camp()
                    == crate::element::Camp::Royalists
                    || !self.civilian_alert_soldier(sim, assets, owner, false)
                {
                    let center = self.reporting_civilian_mut(owner).base.seek_position;
                    self.execute_ai_panic(
                        sim,
                        assets,
                        owner,
                        Some(center),
                        AI_STANDARD_PANIC_RUNS as u8,
                        crate::ai::AlertLevel::Red,
                    );
                }
            }
            _ => {}
        }
    }

    /// Select in registry order, then finish the route before evaluating its result.
    pub(in crate::engine) fn civilian_alert_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        check_door_path: bool,
    ) -> bool {
        let Some(target) = self.select_civilian_alert_soldier(assets, owner, check_door_path)
        else {
            return false;
        };
        self.reporting_state(
            sim,
            assets,
            owner,
            Substate::SeekingCivilianRunningToSoldier,
        );
        self.reporting_civilian_mut(owner).base.antagonist =
            Some(crate::ai::AiEntityHandle::new(target.index()));
        self.approach_reporting_soldier(sim, assets, owner);
        if std::mem::take(&mut self.reporting_civilian_mut(owner).base.couldnt_reachpoint) {
            if !check_door_path {
                return self.civilian_alert_soldier(sim, assets, owner, true);
            }
            self.clear_civilian_alert_friends(owner);
            return false;
        }
        self.execute_ai_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt {
                remark: Remark::CivPanic,
                flags: 0,
            },
        );

        true
    }

    fn select_civilian_alert_soldier(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        check_door_path: bool,
    ) -> Option<EntityId> {
        let camp = self.expect_entity(owner, "civilian alert owner").camp();
        let count = self.world.soldier_registry.camp(camp).len();
        let mut best = None;
        let mut best_distance = u32::MAX;
        for index in 0..count {
            let handle = self.world.soldier_registry.camp(camp)[index];
            let target = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            let Entity::Soldier(soldier) = self.expect_entity(target, "civilian alert registry")
            else {
                unreachable!("soldier ID resolves to non-soldier")
            };
            if soldier.camp() != camp
                || !soldier.is_able_to_fight()
                || soldier
                    .npc
                    .ai_brain
                    .base()
                    .expect("alert candidate lacks AI")
                    .ai_is_script_locked()
            {
                continue;
            }
            if !check_door_path {
                append_detectable(
                    &mut self
                        .world
                        .entities
                        .expect_entity_mut(owner, format_args!("alert friend registration"))
                        .npc_data_mut()
                        .expect("civilian lacks NPC data")
                        .detectable_lists[crate::element::DetectableType::Friend as usize],
                    target,
                    crate::element::DetectableType::Friend,
                    true,
                );
            }
            match self
                .world
                .entities
                .expect_ai_controller(target, format_args!("alert candidate"))
                .current_state
            {
                AiState::Default => {
                    let source = self
                        .expect_entity(owner, "alert distance owner")
                        .element_data();
                    let destination = self
                        .expect_entity(target, "alert distance candidate")
                        .element_data();
                    let here = source.position();
                    let there = destination.position();
                    let mut distance = (there.x - here.x)
                        .abs()
                        .max(
                            ((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                                .abs(),
                        )
                        .max((there.z - here.z).abs())
                        as u32;
                    if source.layer() != destination.layer() {
                        distance = distance.wrapping_add(1000);
                    }
                    if distance < best_distance
                        && (!check_door_path || self.civilian_alert_route_authorized(owner, target))
                    {
                        best = Some(target);
                        best_distance = distance;
                    }
                }
                AiState::Attacking | AiState::Menacing | AiState::Fleeing => {
                    if self.patrol_member_visible(assets, owner, target) {
                        self.clear_civilian_alert_friends(owner);
                        return None;
                    }
                }
                _ => {}
            }
        }
        if best.is_none() {
            self.clear_civilian_alert_friends(owner);
        }
        best
    }

    fn clear_civilian_alert_friends(&mut self, owner: EntityId) {
        self.world
            .entities
            .expect_entity_mut(owner, format_args!("alert friend cleanup"))
            .npc_data_mut()
            .expect("civilian lacks NPC data")
            .detectable_lists[crate::element::DetectableType::Friend as usize]
            .clear();
    }

    fn civilian_alert_route_authorized(&self, owner: EntityId, target: EntityId) -> bool {
        let source = self
            .expect_entity(owner, "alert route owner")
            .element_data();
        let destination = self
            .expect_entity(target, "alert route target")
            .element_data();
        if source.sector() == destination.sector() {
            return true;
        }
        let source_sector = source.sector().expect("alert route owner has no sector");
        let destination_sector = destination
            .sector()
            .expect("alert route target has no sector");
        let here = source.position_map();
        let there = destination.position_map();
        let auth = crate::gate::ActorAuthInfo {
            kind: crate::element::ElementKind::ActorCivilian,
            pc_auth_bit: 0,
            has_lockpick: false,
            has_climb: false,
            has_jump: false,
            is_rider: false,
            posture: source.posture(),
        };
        crate::gate::find_path_gates(
            &self.script_domains.interactables.doors,
            (here.x, here.y),
            u16::from(source_sector),
            (there.x, there.y),
            u16::from(destination_sector),
            Some(&auth),
            false,
            &|sector| self.building_sector_is_authorized(sector),
            &|sector| {
                self.world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&sector)
                    .and_then(|&index| self.world.fast_grid.level.sectors.get(index))
                    .and_then(|sector| sector.lift_type)
            },
        )
        .is_some()
    }

    pub(super) fn reporting_civilian_mut(
        &mut self,
        owner: EntityId,
    ) -> &mut crate::ai_friendly::FriendlyAi {
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
        self.duty_set_state(sim, assets, owner, AiState::Seeking, substate);
    }

    fn clear_reporting_friends(&mut self, owner: EntityId) {
        self.execute_ai_delete_detectable_type(owner, crate::element::DetectableType::Friend);
    }

    fn civilian_call_alert(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        reached: bool,
    ) {
        let frame = self.control.frame_counter;
        let target = self.reporting_target(owner);
        let accepted = self.execute_ai_callback(
            sim,
            assets,
            target,
            &Stimulus::with_human(StimulusType::CallAlert, owner.index()),
        );
        if !accepted {
            self.execute_ai_panic(
                sim,
                assets,
                owner,
                None,
                AI_STANDARD_PANIC_RUNS as u8,
                crate::ai::AlertLevel::Red,
            );

            return;
        }
        if reached {
            self.clear_reporting_friends(owner);
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
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: Remark::CivCallsSoldier,
                    flags: 0,
                },
            );

            self.approach_reporting_soldier(sim, assets, owner);
            self.reporting_civilian_mut(owner)
                .base
                .launch_timer(20, frame);
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
        let passing_door = selected_pass_door_movement(
            &self.world.entities,
            &self.orders.sequence_manager,
            target,
        )
        .is_some();
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

    fn alert_fixture() -> (EngineInner, LevelAssets, EntityId, [EntityId; 2]) {
        let (mut engine, owner, first) = reporting_pair(Substate::DefaultOnPost);
        let second = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        let (sector, _) = crate::engine::test_support::extra_engine_combat::square_sector_map(
            &mut engine,
            (128, 128),
            (2000.0, 2000.0),
        );
        for (id, x) in [(owner, 100.0), (first, 250.0), (second, 400.0)] {
            let entity = engine.world.entities.get_mut(id).unwrap();
            entity.element_data_mut().active = true;
            entity
                .element_data_mut()
                .set_position_map(MapPoint::new(x, 100.0));
            entity.element_data_mut().set_sector(Some(sector));
            entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
            let npc = entity.npc_data_mut().unwrap();
            npc.life_points = 100;
            npc.view_radius = 500;
            npc.view_radius_base = 500;
            npc.view_radius_goal = 500;
            let ai = npc.ai_brain.base_mut().unwrap();
            ai.current_state = AiState::Default;
            ai.current_substate = Substate::DefaultOnPost;
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .civilians
            .push(crate::profiles::CivilianProfile::default());
        (engine, assets, owner, [first, second])
    }

    #[test]
    fn alert_route_finishes_before_emitting_success_remark() {
        let (mut engine, assets, owner, [first, _]) = alert_fixture();
        let sim = crate::sim_rng::test_context();
        assert!(engine.civilian_alert_soldier(&sim, &assets, owner, false));
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(
            ai.base.antagonist,
            Some(crate::ai::AiEntityHandle::new(first.index()))
        );
        assert_eq!(ai.base.current_remark, Remark::CivPanic);
        assert!(!ai.base.couldnt_reachpoint);
    }

    #[test]
    fn alert_retry_consumes_route_failure_before_returning_to_caller() {
        let (mut engine, assets, owner, [first, _]) = alert_fixture();
        engine.reporting_civilian_mut(owner).base.couldnt_reachpoint = true;
        let sim = crate::sim_rng::test_context();
        assert!(engine.civilian_alert_soldier(&sim, &assets, owner, false));
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(
            ai.base.antagonist,
            Some(crate::ai::AiEntityHandle::new(first.index()))
        );
        assert_eq!(ai.base.current_remark, Remark::CivPanic);
        assert!(!ai.base.couldnt_reachpoint);
        let friends = &engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Friend as usize];
        assert_eq!(
            friends.len(),
            2,
            "door-path retry must not append the registry again"
        );
    }

    #[test]
    fn failed_reaction_alert_completes_panic_without_route_failure_event() {
        let (mut engine, assets, owner, soldiers) = alert_fixture();
        for soldier in soldiers {
            engine
                .world
                .entities
                .expect_ai_controller_mut(soldier, format_args!("locked route candidate"))
                .script_locked = true;
        }
        let position = engine.live_ai_position(owner);
        let ai = engine.reporting_civilian_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingCivilianBodyReactiontime;
        ai.base.seek_position = crate::ai::Position {
            x: 200.0,
            y: 100.0,
            ..position
        };
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_remark, Remark::CivPanic);
        assert!(!ai.base.couldnt_reachpoint);
    }

    #[test]
    fn alert_selection_reads_world_distance_and_live_layer_without_entity_views() {
        let (mut engine, assets, owner, [first, second]) = alert_fixture();
        engine
            .world
            .entities
            .get_mut(second)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(100.0, 200.0, 0.0));
        engine
            .world
            .entities
            .get_mut(first)
            .unwrap()
            .element_data_mut()
            .set_position_map_preserving_3d(MapPoint::new(1600.0, 1600.0));
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            Some(first)
        );
        engine
            .world
            .entities
            .get_mut(first)
            .unwrap()
            .element_data_mut()
            .set_layer(1);
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            Some(second)
        );
    }

    #[test]
    fn alert_selection_preserves_registry_ties_and_duplicate_friend_order() {
        let (mut engine, assets, owner, [first, second]) = alert_fixture();
        let position = engine
            .world
            .entities
            .get(first)
            .unwrap()
            .element_data()
            .position();
        engine
            .world
            .entities
            .get_mut(second)
            .unwrap()
            .element_data_mut()
            .set_position(position);
        engine
            .world
            .soldier_registry
            .rebuild_from_order(&engine.world.entities, [second, first]);
        for _ in 0..2 {
            assert_eq!(
                engine.select_civilian_alert_soldier(&assets, owner, false),
                Some(second)
            );
        }
        let friends = &engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Friend as usize];
        assert_eq!(
            friends
                .iter()
                .map(|friend| friend.element.unwrap())
                .collect::<Vec<_>>(),
            vec![second, first, second, first]
        );
        engine.select_civilian_alert_soldier(&assets, owner, true);
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .npc_data()
                .unwrap()
                .detectable_lists[crate::element::DetectableType::Friend as usize]
                .len(),
            4
        );
    }

    #[test]
    fn alert_selection_excludes_other_camps_and_script_locked_soldiers() {
        let (mut engine, assets, owner, [first, second]) = alert_fixture();
        let Entity::Soldier(soldier) = engine.world.entities.get_mut(first).unwrap() else {
            unreachable!()
        };
        soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        engine
            .world
            .soldier_registry
            .rebuild_from_order(&engine.world.entities, [first, second]);
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            Some(second)
        );
        engine
            .world
            .entities
            .expect_ai_controller_mut(second, format_args!("locked alert candidate"))
            .script_locked = true;
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            None
        );
        assert!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .npc_data()
                .unwrap()
                .detectable_lists[crate::element::DetectableType::Friend as usize]
                .is_empty()
        );
    }

    #[test]
    fn alerted_friend_uses_current_activity_and_real_view_radius() {
        let (mut engine, assets, owner, [first, second]) = alert_fixture();
        engine
            .world
            .entities
            .expect_ai_controller_mut(second, format_args!("alerted friend"))
            .current_state = AiState::Attacking;
        engine
            .world
            .entities
            .get_mut(second)
            .unwrap()
            .element_data_mut()
            .active = false;
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            Some(first)
        );
        engine
            .world
            .entities
            .get_mut(second)
            .unwrap()
            .element_data_mut()
            .active = true;
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .view_radius = 1;
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            Some(first)
        );
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .view_radius = 500;
        assert_eq!(
            engine.select_civilian_alert_soldier(&assets, owner, false),
            None
        );
        assert!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .npc_data()
                .unwrap()
                .detectable_lists[crate::element::DetectableType::Friend as usize]
                .is_empty()
        );
    }

    #[test]
    fn civilian_patrol_reads_live_chief_for_walk_run_and_backwards_facing() {
        for (distance, prior, expected) in [
            (
                45.0,
                Substate::DefaultOnPost,
                Substate::DefaultPatrolEnroute,
            ),
            (
                60.0,
                Substate::DefaultOnPost,
                Substate::DefaultPatrolEnrouteRunning,
            ),
            (
                45.0,
                Substate::DefaultPatrolEnroute,
                Substate::DefaultPatrolEnroute,
            ),
            (
                -10.0,
                Substate::DefaultPatrolEnroute,
                Substate::DefaultPatrolEnroute,
            ),
        ] {
            let (mut engine, assets, owner, [chief, _]) = alert_fixture();
            let position = engine.live_ai_position(owner);
            let ai = engine.reporting_civilian_mut(owner);
            ai.base.patrol_chief = Some(chief);
            ai.base.current_substate = prior;
            ai.base.current_music_alert_status = crate::ai::AlertLevel::Yellow;
            ai.base.view_alert_status = crate::ai::AlertLevel::Yellow;
            engine.execute_friendly_callback(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::with_position(
                    StimulusType::CallPatrolCoordinate,
                    crate::ai::Position {
                        x: position.x + distance,
                        ..position
                    },
                ),
            );
            let ai = engine.reporting_civilian_mut(owner);
            assert_eq!(ai.base.current_substate, expected);
            if distance > 0.0 {
                assert_eq!(
                    ai.base.current_music_alert_status,
                    crate::ai::AlertLevel::Green
                );
                assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Green);
                let order = if distance > 50.0 {
                    crate::order::OrderType::RunningUpright
                } else {
                    crate::order::OrderType::WalkingUpright
                };
                assert!(engine.orders.sequence_manager.sequences_iter().any(|sequence|
                    sequence.elements.iter().any(|element| element.owner == Some(owner)
                        && matches!(element.data, crate::sequence::SequenceElementData::Movement { action, .. } if action == order))));
            } else {
                assert!(!ai.base.already_on_point);
                assert!(
                    engine
                        .orders
                        .sequence_manager
                        .sequences_iter()
                        .all(
                            |sequence| sequence.elements.iter().all(|element| element.owner
                                != Some(owner)
                                || !matches!(
                                    element.data,
                                    crate::sequence::SequenceElementData::Movement { .. }
                                ))
                        )
                );
            }
        }
    }

    #[test]
    fn patrol_waiting_queries_chief_state_at_delivery() {
        let (mut engine, assets, owner, [chief, _]) = alert_fixture();
        engine.reporting_civilian_mut(owner).base.patrol_chief = Some(chief);
        engine.reporting_civilian_mut(owner).base.current_substate =
            Substate::DefaultPatrolEnrouteWaiting;
        let frame = engine.control.frame_counter;
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
        assert!(engine.reporting_civilian_mut(owner).base.timer_is_running);
        assert_eq!(
            engine
                .reporting_civilian_mut(owner)
                .base
                .when_does_timer_ring,
            frame + 200
        );
    }

    #[test]
    #[should_panic(expected = "waiting civilian requires a patrol chief")]
    fn patrol_waiting_requires_live_chief() {
        let (mut engine, assets, owner, _) = alert_fixture();
        engine.reporting_civilian_mut(owner).base.current_substate =
            Substate::DefaultPatrolEnrouteWaiting;
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
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
        assert_eq!(
            engine.execute_friendly_callback(
                &crate::sim_rng::test_context(),
                &LevelAssets::default(),
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            ),
            Some(false)
        );
        let ai = engine.reporting_civilian_mut(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingCivilianGiveAlertingReportToSoldierStart
        );
        assert!(ai.base.timer_is_running);
    }

    #[test]
    fn reporting_family_preserves_general_event_routing() {
        let (mut engine, owner, target) =
            reporting_pair(Substate::SeekingCivilianRunningToSoldierSeen);
        for event in [
            StimulusType::EventReturnToDuty,
            StimulusType::EventLoseConsciousness,
            StimulusType::EventView,
            StimulusType::EventCouldntReachPoint,
        ] {
            let stimulus = if event == StimulusType::EventView {
                Stimulus::with_human(event, target.index())
            } else {
                Stimulus::new(event)
            };
            assert_eq!(
                engine.execute_friendly_callback(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::default(),
                    owner,
                    &stimulus,
                ),
                if matches!(
                    event,
                    StimulusType::EventView | StimulusType::EventLoseConsciousness
                ) {
                    Some(false)
                } else {
                    None
                }
            );
        }
    }

    #[test]
    fn report_point_done_faces_live_target_then_launches_timer() {
        let (mut engine, owner, _) =
            reporting_pair(Substate::SeekingCivilianGiveAlertingReportToSoldierPoint);
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 0.0);
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_direction_instantly(direction as i16);
        entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
        engine.execute_friendly_callback(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
            &Stimulus::new(StimulusType::EventDone),
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
