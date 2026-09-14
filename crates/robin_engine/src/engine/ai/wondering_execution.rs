use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, EmoticonType, GotoFlags, Position, Remark, ReportType,
    SpeechFlags, Stimulus, StimulusType, Substate,
};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::element::Element as _;
use crate::profiles::{CivilianType, ProfileRank};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_wondering_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        if !matches!(
            stimulus.stimulus_type,
            EventReachPoint
                | EventDone
                | EventTimer
                | EventSyncCharly
                | CallCoordinate
                | CallInstruction
                | CallReport
                | EventGaloppLoopEnd
                | EventMyTalk0
                | EventMyTalk1
                | EventMyTalk2
                | EventMyTalk3
                | CallYourTalk0
                | CallYourTalk1
                | CallYourTalk2
                | CallYourTalk3
        ) {
            return None;
        }
        let substate = self.seek_enemy(owner).base.current_substate;
        if !matches!(
            substate,
            Substate::WonderingAppleSauceInTheVisor
                | Substate::WonderingAppleReactiontime
                | Substate::WonderingAppleChasingChild
                | Substate::WonderingAppleChasingChildWaiting
                | Substate::WonderingAppleChasingChildEnd
                | Substate::WonderingHeardWhistling
                | Substate::WonderingWatchingWhistling
        ) {
            return None;
        }
        match (substate, stimulus.stimulus_type) {
            (Substate::WonderingAppleSauceInTheVisor, EventTimer) => {
                let unconscious = self
                    .expect_entity(owner, "apple visor owner")
                    .is_unconscious();
                self.feedback.titbit_manager.remove_unconscious_stars_if(
                    crate::titbit::ElementHandle(owner.index()),
                    unconscious,
                );
                self.execute_apple_anger(sim, assets, owner);
            }
            (Substate::WonderingAppleReactiontime, EventTimer) => {
                let ai = self.seek_enemy(owner);
                let drunk =
                    ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT;
                let entity = self.expect_entity(owner, "apple reaction owner");
                let outside = entity.element_data().active
                    && !self.entity_data_in_building_sector(entity.element_data());
                let react = drunk || outside && ai.soldier_profile_apple > 0;
                if !react || !self.chase_live_children(sim, assets, owner) {
                    self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
                }
            }
            (Substate::WonderingAppleChasingChild, EventMyTalk1) => {
                let target = self.wondering_antagonist(owner);
                self.execute_ai_callback(sim, assets, target, &Stimulus::new(CallYourTalk1));
            }
            (Substate::WonderingAppleChasingChild, EventTimer) => {
                if self.seek_enemy(owner).base.lasting_panic_runs > 0 {
                    self.seek_enemy_mut(owner).base.lasting_panic_runs -= 1;
                    self.refresh_child_chase(sim, assets, owner);
                    self.wondering_timer(owner, 10);
                } else {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingAppleChasingChildEnd,
                    );
                    let target = self.wondering_antagonist(owner);
                    self.wondering_face_entity(sim, assets, owner, target);
                    self.wondering_timer(owner, 30);
                }
            }
            (Substate::WonderingAppleChasingChild, EventReachPoint) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingAppleChasingChildWaiting,
                );
                self.wondering_timer(owner, 10);
            }
            (Substate::WonderingAppleChasingChildWaiting, EventTimer) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingAppleChasingChild,
                );
                self.wondering_timer(owner, 1);
            }
            (Substate::WonderingAppleChasingChildEnd, EventTimer) => {
                self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty())
            }
            (Substate::WonderingHeardWhistling, EventTimer) => {
                let position = self.seek_enemy(owner).base.seek_position;
                self.wondering_face_position(sim, assets, owner, position);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingWatchingWhistling,
                );
                self.wondering_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
            }
            (Substate::WonderingWatchingWhistling, EventTimer) => {
                self.follow_live_whistle(sim, assets, owner)
            }
            _ => {}
        }
        Some(false)
    }

    fn wondering_timer(&mut self, owner: EntityId, duration: u32) {
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner)
            .base
            .launch_timer(duration, frame);
    }

    fn wondering_antagonist(&self, owner: EntityId) -> EntityId {
        let handle = self
            .seek_enemy(owner)
            .base
            .antagonist
            .expect("child chase requires antagonist");
        self.expect_human_id_for_ai_handle(handle.get(), "child chase antagonist")
    }

    fn wondering_face_entity(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let position = self.live_ai_position(target);
        let elevation = self
            .expect_entity(target, "wondering facing target")
            .position_iface()
            .get_elevation() as i16;
        let body = self
            .expect_entity(owner, "wondering facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            position.x - body.x,
            (position.y - (body.y - body.z)) + (elevation as f32 - body.z),
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }

    fn wondering_face_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        let target = crate::ai::ai_position_to_point_3d(
            &self.world.fast_grid,
            self.sight_obstacles(assets),
            position,
        );
        let body = self
            .expect_entity(owner, "wondering facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
    }

    fn refresh_child_chase(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let target = self.wondering_antagonist(owner);
        let position = self.live_ai_position(target);
        self.duty_go_near(
            sim,
            assets,
            owner,
            position,
            5,
            GotoFlags::RUN | GotoFlags::DONT_STOP,
        );
    }

    fn chase_live_children(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        self.seek_enemy_mut(owner).base.antagonist = None;
        let mut suspects = Vec::new();
        let mut nearest = 65_432_u16;
        // Membership must survive callbacks to earlier children in this list.
        for target in self.one_shot_noise_listener_ids() {
            let Entity::Civilian(child) = self.expect_entity(target, "child registry") else {
                continue;
            };
            if child.civilian.cached_civilian_type != CivilianType::Child
                || child.is_dead()
                || child.is_unconscious()
                || child
                    .npc
                    .ai_brain
                    .base()
                    .expect("child requires AI")
                    .current_state
                    != AiState::Default
            {
                continue;
            }
            if !self.npc_is_detecting_human(assets, owner, target, self.control.frame_counter) {
                continue;
            }
            suspects.push(target);
            let here = self
                .expect_entity(owner, "child chase origin")
                .element_data()
                .position();
            let there = self
                .expect_entity(target, "child chase candidate")
                .element_data()
                .position();
            let distance = (there.x - here.x)
                .abs()
                .max(((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
                as u32 as u16;
            if distance < nearest {
                nearest = distance;
                self.seek_enemy_mut(owner).base.antagonist =
                    Some(AiEntityHandle::new(target.index()));
            }
        }
        if suspects.is_empty() {
            return false;
        }
        assert!(
            self.seek_enemy(owner).base.antagonist.is_some(),
            "visible child requires nearest target"
        );
        for target in suspects {
            let event = if self.seek_enemy(owner).base.antagonist
                == Some(AiEntityHandle::new(target.index()))
            {
                StimulusType::CallYouJustWait
            } else {
                StimulusType::EventAppleChaseNear
            };
            self.execute_ai_callback(
                sim,
                assets,
                target,
                &Stimulus::with_human(event, owner.index()),
            );
        }
        let ai = self.seek_enemy_mut(owner);
        ai.base.lasting_panic_runs = (ai.soldier_profile_apple / 2) as u8;
        ai.base.set_emoticon(EmoticonType::Thunderstorm);
        ai.base
            .say_with_flags(Remark::ChasesChild, SpeechFlags::MYTALK_1);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingAppleChasingChild,
        );
        self.refresh_child_chase(sim, assets, owner);
        self.wondering_timer(owner, 10);
        true
    }

    fn execute_apple_anger(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self.seek_enemy_mut(owner);
        if ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || ai.new_task_priority < ai.current_task_priority
        {
            return;
        }
        ai.current_task_priority = ai.new_task_priority;
        if let Some(object) = ai.base.object_of_desire.take() {
            ai.base.forgotten_objects.push(object.get());
        }
        ai.base.stop_all();
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingAppleReactiontime,
        );
        let ai = self.seek_enemy_mut(owner);
        ai.base.say(if ai.is_vip {
            Remark::VipAppleNo
        } else {
            Remark::HitByApple
        });
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        let position = self.seek_enemy(owner).base.seek_position;
        self.wondering_face_position(sim, assets, owner, position);
        self.seek_enemy_mut(owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);
        self.wondering_timer(owner, 50);
    }

    fn follow_live_whistle(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self.seek_enemy(owner);
        let entity = self.expect_entity(owner, "whistle owner");
        let outside = entity.element_data().active
            && !self.entity_data_in_building_sector(entity.element_data());
        if ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !outside
            || ai.soldier_profile_whistle <= 1
            || ai.company_number == 100
        {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        let officer = if ai.soldier_profile_rank == ProfileRank::Soldier {
            self.live_whistle_officer(assets, owner)
        } else {
            None
        };
        let ai = self.seek_enemy(owner);
        let send_soldier = ai.soldier_profile_rank == ProfileRank::Officer
            && (ai.soldier_profile_initiative < 50 || !ai.base.patrol.is_empty());
        if let Some(officer) = officer {
            self.wondering_face_entity(sim, assets, owner, officer);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Default,
                Substate::DefaultLookingOfficerForAdvice,
            );
            self.seek_enemy_mut(owner)
                .base
                .set_emoticon(EmoticonType::QuestionMark);
            self.wondering_timer(owner, 100);
        } else if send_soldier {
            self.execute_ai_officer_look_for_soldier(sim, assets, owner, ReportType::Noise);
        } else {
            let ai = self.seek_enemy(owner);
            let position = ai.base.seek_position;
            let radius = (400 * (ai.soldier_profile_whistle as u32 - 2) / 98) as u16;
            self.execute_ai_seek_area(
                sim,
                assets,
                owner,
                position,
                radius,
                SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                UNDEFINED_DIRECTION,
            );
        }
    }

    fn live_whistle_officer(&self, assets: &LevelAssets, owner: EntityId) -> Option<EntityId> {
        let viewer = self.expect_entity(owner, "whistle advice viewer");
        let seek = self.seek_enemy(owner).base.seek_position;
        for &handle in self.ai.global.all_soldier_handles.iter() {
            let target = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            let Entity::Soldier(soldier) = self.expect_entity(target, "whistle officer registry")
            else {
                unreachable!()
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("officer requires enemy AI");
            if soldier.camp() != viewer.camp()
                || ai.soldier_profile_rank != ProfileRank::Officer
                || !soldier.is_able_to_fight()
            {
                continue;
            }
            if !viewer.element_data().active
                || !soldier.element.active
                || !patrol_member_visible_from_raw_world(
                    viewer.element_data().position(),
                    viewer.soldier_data().is_some_and(|s| s.rider),
                    viewer
                        .ai_actor_data()
                        .expect("viewer requires NPC")
                        .view_radius,
                    self.entity_data_in_building_sector(viewer.element_data()),
                    soldier.element.position(),
                    soldier.element.posture(),
                    soldier.soldier.rider,
                    soldier.element.direction(),
                    self.entity_data_in_building_sector(&soldier.element),
                    self.sight_obstacles(assets),
                )
            {
                continue;
            }
            if !ai.base.ai_is_script_locked()
                && ai.base.seek_position.x == seek.x
                && ai.base.seek_position.y == seek.y
                && ai.base.seek_position.level == seek.level
            {
                return Some(target);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::battle_decision_observation_tests::fixture;
    use super::*;

    #[test]
    fn child_chase_refresh_keeps_running_state_until_actual_arrival() {
        let (mut engine, assets, owner, target) = fixture(false);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingAppleChasingChild;
        ai.base.antagonist = Some(AiEntityHandle::new(target.index()));
        ai.base.lasting_panic_runs = 2;
        let target_position = engine.live_ai_position(target);
        let sim = crate::sim_rng::test_context();
        assert_eq!(
            engine.execute_ai_wondering_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer)
            ),
            Some(false)
        );
        let ai = engine.seek_enemy(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingAppleChasingChild
        );
        assert_eq!(ai.base.lasting_panic_runs, 1);
        assert_eq!(ai.base.last_goto_destination, target_position);
        assert_eq!(ai.base.when_does_timer_ring, 110);
        assert_eq!(
            engine.execute_ai_wondering_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            ),
            Some(false)
        );
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::WonderingAppleChasingChildWaiting
        );
        assert_eq!(
            engine.execute_ai_wondering_event(
                &sim,
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer),
            ),
            Some(false)
        );
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::WonderingAppleChasingChild
        );
        assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 101);
    }

    #[test]
    fn wondering_route_leaves_interruptions_for_normal_dispatch() {
        let (mut engine, assets, owner, _) = fixture(false);
        engine.seek_enemy_mut(owner).base.current_state = AiState::Wondering;
        for substate in [
            Substate::WonderingAppleSauceInTheVisor,
            Substate::WonderingAppleReactiontime,
            Substate::WonderingAppleChasingChild,
            Substate::WonderingAppleChasingChildWaiting,
            Substate::WonderingAppleChasingChildEnd,
            Substate::WonderingHeardWhistling,
            Substate::WonderingWatchingWhistling,
        ] {
            engine.seek_enemy_mut(owner).base.current_substate = substate;
            for event in [
                StimulusType::CallAlert,
                StimulusType::EventCouldntReachPoint,
                StimulusType::EventOutOfView,
            ] {
                assert_eq!(
                    engine.execute_ai_wondering_event(
                        &crate::sim_rng::test_context(),
                        &assets,
                        owner,
                        &Stimulus::new(event)
                    ),
                    None
                );
                assert_eq!(engine.seek_enemy(owner).base.current_substate, substate);
            }
        }
    }

    #[test]
    fn apple_chase_without_visible_children_clears_previous_antagonist() {
        let (mut engine, assets, owner, target) = fixture(false);
        engine.seek_enemy_mut(owner).base.antagonist = Some(AiEntityHandle::new(target.index()));
        assert!(!engine.chase_live_children(&crate::sim_rng::test_context(), &assets, owner));
        assert_eq!(engine.seek_enemy(owner).base.antagonist, None);
    }

    #[test]
    fn exhausted_child_chase_faces_target_and_starts_final_wait() {
        let (mut engine, assets, owner, target) = fixture(false);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingAppleChasingChild;
        ai.base.antagonist = Some(AiEntityHandle::new(target.index()));
        ai.base.lasting_panic_runs = 0;
        assert_eq!(
            engine.execute_ai_wondering_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventTimer),
            ),
            Some(false)
        );
        assert_eq!(
            engine.seek_enemy(owner).base.current_substate,
            Substate::WonderingAppleChasingChildEnd
        );
        assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 130);
    }
}
