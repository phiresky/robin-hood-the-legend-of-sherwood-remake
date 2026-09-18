use super::*;
use crate::ai::{
    AiEntityHandle, AiState, DutyFlags, EmoticonType, GotoFlags, Position, Remark, ReportType,
    SpeechFlags, Stimulus, StimulusType, Substate,
};
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::element::Element as _;
use crate::profiles::{CivilianType, ProfileRank};
#[cfg(test)]
use crate::sim_rng::SimulationContext;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_wondering_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_wondering_event(stimulus)
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

    #[cfg(test)]
    fn chase_live_children(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        AiOwnerCtx::new(self, sim, assets, owner).chase_live_children()
    }

    fn live_whistle_officer(&self, assets: &LevelAssets, owner: EntityId) -> Option<EntityId> {
        let viewer = self.expect_entity(owner, "whistle advice viewer");
        let seek = self.seek_enemy(owner).base.seek_position;
        for &handle in self.world.soldier_registry.camp(viewer.camp()) {
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
                || ai.profile(&assets.profile_manager).rank != ProfileRank::Officer
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

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_wondering_event(
        &mut self,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        use StimulusType::*;
        if !stimulus.stimulus_type.is_expected_class() {
            return None;
        }
        let substate = self.engine.seek_enemy(self.owner).base.current_substate;
        if !matches!(
            substate,
            Substate::WonderingAppleSauceInTheVisor
                | Substate::WonderingAppleReactiontime
                | Substate::WonderingAppleChasingChild
                | Substate::WonderingAppleChasingChildWaiting
                | Substate::WonderingAppleChasingChildEnd
                | Substate::WonderingHeardWhistling
                | Substate::WonderingWatchingWhistling
                | Substate::SeekingHeardstepsPreReactiontime
                | Substate::SeekingHeardstepsReactiontime
                | Substate::SeekingHeardsteps
                | Substate::SeekingJustWatching
                | Substate::SeekingJustWatchingSidewards
        ) {
            return None;
        }
        match (substate, stimulus.stimulus_type) {
            (Substate::SeekingHeardstepsPreReactiontime, EventTimer) => {
                self.execute_noise_pre_reaction();
            }
            (Substate::SeekingHeardstepsReactiontime, EventTimer) => {
                let officer = if self
                    .engine
                    .seek_enemy(self.owner)
                    .profile(&self.assets.profile_manager)
                    .rank
                    == ProfileRank::Soldier
                {
                    self.engine.live_whistle_officer(self.assets, self.owner)
                } else {
                    None
                };
                if officer.is_some() {
                    self.duty_set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
                    self.engine.wondering_timer(self.owner, 100);
                } else {
                    self.duty_set_state(AiState::Seeking, Substate::SeekingHeardsteps);
                    let ai = self.engine.seek_enemy(self.owner);
                    let destination = ai.base.seek_position;
                    let flags = if ai.investigating_distraction {
                        GotoFlags::RUN
                    } else {
                        GotoFlags::empty()
                    };
                    self.duty_go_to(destination, flags);
                    self.engine.wondering_timer(self.owner, 200);
                }
            }
            (Substate::SeekingHeardsteps, EventReachPoint | EventTimer) => {
                let center = self.engine.live_ai_position(self.owner);
                self.execute_ai_seek_area(
                    center,
                    0,
                    SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                    UNDEFINED_DIRECTION,
                );
            }
            (Substate::SeekingJustWatching, EventTimer) => {
                self.duty_set_state(AiState::Seeking, Substate::SeekingJustWatchingSidewards);
                let direction =
                    if crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::EnemySeekLook, 0..2)
                        != 0
                    {
                        crate::ai::LookDirection::RightLeft
                    } else {
                        crate::ai::LookDirection::LeftRight
                    };
                self.execute_ai_look_sidewards(direction);
            }
            (Substate::SeekingJustWatchingSidewards, EventDone) => {
                match self
                    .engine
                    .seek_enemy(self.owner)
                    .profile(&self.assets.profile_manager)
                    .rank
                {
                    ProfileRank::Soldier => self.execute_ai_return_to_duty(DutyFlags::empty()),
                    ProfileRank::Officer => {
                        self.execute_ai_officer_look_for_soldier(ReportType::Noise)
                    }
                    ProfileRank::Knight | ProfileRank::None => {}
                }
            }
            (Substate::WonderingAppleSauceInTheVisor, EventTimer) => {
                let unconscious = self
                    .engine
                    .expect_entity(self.owner, "apple visor owner")
                    .is_unconscious();
                self.engine
                    .feedback
                    .titbit_manager
                    .remove_unconscious_stars_if(
                        crate::titbit::ElementHandle(self.owner.index()),
                        unconscious,
                    );
                self.execute_apple_anger();
            }
            (Substate::WonderingAppleReactiontime, EventTimer) => {
                let ai = self.engine.seek_enemy(self.owner);
                let drunk =
                    ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT;
                let entity = self
                    .engine
                    .expect_entity(self.owner, "apple reaction owner");
                let outside = entity.element_data().active
                    && !self
                        .engine
                        .entity_data_in_building_sector(entity.element_data());
                let react = drunk || outside && ai.profile(&self.assets.profile_manager).apple > 0;
                if !react || !self.chase_live_children() {
                    self.execute_ai_return_to_duty(DutyFlags::empty());
                }
            }
            (Substate::WonderingAppleChasingChild, EventMyTalk1) => {
                let target = self.engine.wondering_antagonist(self.owner);
                self.engine.execute_ai_callback(
                    self.sim,
                    self.assets,
                    target,
                    &Stimulus::new(CallYourTalk1),
                );
            }
            (Substate::WonderingAppleChasingChild, EventTimer) => {
                if self.engine.seek_enemy(self.owner).base.lasting_panic_runs > 0 {
                    self.engine
                        .seek_enemy_mut(self.owner)
                        .base
                        .lasting_panic_runs -= 1;
                    self.refresh_child_chase();
                    self.engine.wondering_timer(self.owner, 10);
                } else {
                    self.duty_set_state(
                        AiState::Wondering,
                        Substate::WonderingAppleChasingChildEnd,
                    );
                    let target = self.engine.wondering_antagonist(self.owner);
                    self.wondering_face_entity(target);
                    self.engine.wondering_timer(self.owner, 30);
                }
            }
            (Substate::WonderingAppleChasingChild, EventReachPoint) => {
                self.duty_set_state(
                    AiState::Wondering,
                    Substate::WonderingAppleChasingChildWaiting,
                );
                self.engine.wondering_timer(self.owner, 10);
            }
            (Substate::WonderingAppleChasingChildWaiting, EventTimer) => {
                self.duty_set_state(AiState::Wondering, Substate::WonderingAppleChasingChild);
                self.engine.wondering_timer(self.owner, 1);
            }
            (Substate::WonderingAppleChasingChildEnd, EventTimer) => {
                self.execute_ai_return_to_duty(DutyFlags::empty())
            }
            (Substate::WonderingHeardWhistling, EventTimer) => {
                let position = self.engine.seek_enemy(self.owner).base.seek_position;
                self.wondering_face_position(position);
                self.duty_set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);
                self.engine
                    .wondering_timer(self.owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
            }
            (Substate::WonderingWatchingWhistling, EventTimer) => self.follow_live_whistle(),
            _ => {}
        }
        Some(false)
    }

    fn execute_noise_pre_reaction(&mut self) {
        let ai = self.engine.seek_enemy(self.owner);
        let decline = if ai.investigating_distraction {
            false
        } else {
            match ai.profile(&self.assets.profile_manager).rank {
                ProfileRank::Officer => {
                    let here = self.engine.live_ai_position(self.owner);
                    let source = ai.base.seek_position;
                    !ai.base.patrol.is_empty()
                        || (here.x - source.x).abs().max((here.y - source.y).abs()) > 100.0
                }
                ProfileRank::Soldier | ProfileRank::Knight => {
                    let entity = self
                        .engine
                        .expect_entity(self.owner, "noise reaction owner");
                    ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                        || !entity.element_data().active
                        || self
                            .engine
                            .entity_data_in_building_sector(entity.element_data())
                        || ai.profile(&self.assets.profile_manager).duty
                        || ai.company_number == 100
                }
                ProfileRank::None => false,
            }
        };
        self.engine
            .seek_enemy_mut(self.owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);
        if decline {
            self.duty_set_state(AiState::Seeking, Substate::SeekingJustWatching);
            let position = self.engine.seek_enemy(self.owner).base.seek_position;
            self.wondering_face_position(position);
        } else {
            let position = self.engine.seek_enemy(self.owner).base.seek_position;
            self.wondering_face_position(position);
            self.duty_set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
        }
        self.engine
            .wondering_timer(self.owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
    }

    fn wondering_face_entity(&mut self, target: EntityId) {
        let position = self.engine.live_ai_position(target);
        let elevation = self
            .engine
            .expect_entity(target, "wondering facing target")
            .position_iface()
            .get_elevation() as i16;
        let body = self
            .engine
            .expect_entity(self.owner, "wondering facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            position.x - body.x,
            (position.y - (body.y - body.z)) + (elevation as f32 - body.z),
        );
        self.duty_face_direction(direction as u16);
    }

    fn wondering_face_position(&mut self, position: Position) {
        let target = crate::ai::ai_position_to_point_3d(
            &self.engine.world.fast_grid,
            self.engine.sight_obstacles(self.assets),
            position,
        );
        let body = self
            .engine
            .expect_entity(self.owner, "wondering facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - body.x,
            target.y - body.y,
        );
        self.duty_face_direction(direction as u16);
    }

    fn refresh_child_chase(&mut self) {
        let target = self.engine.wondering_antagonist(self.owner);
        let position = self.engine.live_ai_position(target);
        self.duty_go_near(position, 5, GotoFlags::RUN | GotoFlags::DONT_STOP);
    }

    fn chase_live_children(&mut self) -> bool {
        self.engine.seek_enemy_mut(self.owner).base.antagonist = None;
        let mut suspects = Vec::new();
        let mut nearest = 65_432_u16;
        let count = self.engine.world.npc_registry_ids.len();
        for index in 0..count {
            let target = self.engine.world.npc_registry_ids[index];
            let Entity::Civilian(child) = self.engine.expect_entity(target, "child registry")
            else {
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
            if !self.engine.npc_is_detecting_human(
                self.assets,
                self.owner,
                target,
                self.engine.control.frame_counter,
            ) {
                continue;
            }
            suspects.push(target);
            let here = self
                .engine
                .expect_entity(self.owner, "child chase origin")
                .element_data()
                .position();
            let there = self
                .engine
                .expect_entity(target, "child chase candidate")
                .element_data()
                .position();
            let distance = (there.x - here.x)
                .abs()
                .max(((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
                as u32 as u16;
            if distance < nearest {
                nearest = distance;
                self.engine.seek_enemy_mut(self.owner).base.antagonist =
                    Some(AiEntityHandle::new(target.index()));
            }
        }
        if suspects.is_empty() {
            return false;
        }
        assert!(
            self.engine.seek_enemy(self.owner).base.antagonist.is_some(),
            "visible child requires nearest target"
        );
        for target in suspects {
            let event = if self.engine.seek_enemy(self.owner).base.antagonist
                == Some(AiEntityHandle::new(target.index()))
            {
                StimulusType::CallYouJustWait
            } else {
                StimulusType::EventAppleChaseNear
            };
            self.engine.execute_ai_callback(
                self.sim,
                self.assets,
                target,
                &Stimulus::with_human(event, self.owner.index()),
            );
        }
        let ai = self.engine.seek_enemy_mut(self.owner);
        ai.base.lasting_panic_runs = (ai.profile(&self.assets.profile_manager).apple / 2) as u8;
        ai.base.set_emoticon(EmoticonType::Thunderstorm);
        self.execute_ai_speech(crate::ai::AiSpeechAttempt {
            remark: Remark::ChasesChild,
            flags: SpeechFlags::MYTALK_1.bits(),
        });

        self.duty_set_state(AiState::Wondering, Substate::WonderingAppleChasingChild);
        self.refresh_child_chase();
        self.engine.wondering_timer(self.owner, 10);
        true
    }

    fn execute_apple_anger(&mut self) {
        let ai = self.engine.seek_enemy_mut(self.owner);
        if ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || ai.new_task_priority < ai.current_task_priority
        {
            return;
        }
        ai.current_task_priority = ai.new_task_priority;
        if let Some(object) = ai.base.object_of_desire.take() {
            ai.base.forgotten_objects.push(object.get());
        }
        self.stop_ai_owner();
        self.duty_set_state(AiState::Wondering, Substate::WonderingAppleReactiontime);
        let remark = if self.engine.seek_enemy(self.owner).is_vip {
            Remark::VipAppleNo
        } else {
            Remark::HitByApple
        };
        self.execute_ai_speech(crate::ai::AiSpeechAttempt { remark, flags: 0 });
        let position = self.engine.seek_enemy(self.owner).base.seek_position;
        self.wondering_face_position(position);
        self.engine
            .seek_enemy_mut(self.owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);
        self.engine.wondering_timer(self.owner, 50);
    }

    fn follow_live_whistle(&mut self) {
        let ai = self.engine.seek_enemy(self.owner);
        let entity = self.engine.expect_entity(self.owner, "whistle owner");
        let outside = entity.element_data().active
            && !self
                .engine
                .entity_data_in_building_sector(entity.element_data());
        if ai.base.blood_alcohol as i32 > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !outside
            || ai.profile(&self.assets.profile_manager).whistle <= 1
            || ai.company_number == 100
        {
            self.execute_ai_return_to_duty(DutyFlags::empty());
            return;
        }
        let officer = if ai.profile(&self.assets.profile_manager).rank == ProfileRank::Soldier {
            self.engine.live_whistle_officer(self.assets, self.owner)
        } else {
            None
        };
        let ai = self.engine.seek_enemy(self.owner);
        let send_soldier = ai.profile(&self.assets.profile_manager).rank == ProfileRank::Officer
            && (ai.profile(&self.assets.profile_manager).initiative < 50
                || !ai.base.patrol.is_empty());
        if let Some(officer) = officer {
            self.wondering_face_entity(officer);
            self.duty_set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
            self.engine
                .seek_enemy_mut(self.owner)
                .base
                .set_emoticon(EmoticonType::QuestionMark);
            self.engine.wondering_timer(self.owner, 100);
        } else if send_soldier {
            self.execute_ai_officer_look_for_soldier(ReportType::Noise);
        } else {
            let ai = self.engine.seek_enemy(self.owner);
            let position = ai.base.seek_position;
            let radius =
                (400 * (ai.profile(&self.assets.profile_manager).whistle as u32 - 2) / 98) as u16;
            self.execute_ai_seek_area(
                position,
                radius,
                SeekFlags::LOCATION_FIRST | SeekFlags::WALKING,
                UNDEFINED_DIRECTION,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::battle_decision_observation_tests::fixture;
    use super::*;

    #[test]
    fn noise_reaction_uses_live_rank_activity_and_current_patrol_membership() {
        for (rank, active, has_patrol, expected) in [
            (
                ProfileRank::None,
                false,
                false,
                Substate::SeekingHeardstepsReactiontime,
            ),
            (
                ProfileRank::Soldier,
                false,
                false,
                Substate::SeekingJustWatching,
            ),
            (
                ProfileRank::Officer,
                true,
                false,
                Substate::SeekingHeardstepsReactiontime,
            ),
            (
                ProfileRank::Officer,
                true,
                true,
                Substate::SeekingJustWatching,
            ),
        ] {
            let (mut engine, mut assets, owner, target) = fixture(false);
            engine
                .get_entity_mut(owner)
                .unwrap()
                .element_data_mut()
                .active = active;
            let position = engine.live_ai_position(owner);
            let ai = engine.seek_enemy_mut(owner);
            ai.base.current_state = AiState::Seeking;
            ai.base.current_substate = Substate::SeekingHeardstepsPreReactiontime;
            crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
                profile.rank = rank
            });
            crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
                profile.duty = false
            });
            ai.base.seek_position = Position {
                x: position.x + 50.0,
                ..position
            };
            ai.base.theoretical_patrol.push(target);
            ai.base.missed_patrol_members.push(target);
            if has_patrol {
                ai.base.patrol.push(target);
            }
            assert_eq!(
                engine.execute_ai_wondering_event(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventTimer)
                ),
                Some(false)
            );
            assert_eq!(engine.seek_enemy(owner).base.current_substate, expected);
            assert_eq!(engine.seek_enemy(owner).base.when_does_timer_ring, 160);
        }
    }

    #[test]
    fn distraction_keeps_running_investigation_through_live_noise_handlers() {
        let (mut engine, mut assets, owner, target) = fixture(false);
        let destination = engine.live_ai_position(target);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingHeardstepsPreReactiontime;
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.rank = ProfileRank::Soldier
        });
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.duty = true
        });
        ai.investigating_distraction = true;
        ai.base.seek_position = destination;
        let sim = crate::sim_rng::test_context();
        for expected in [
            Substate::SeekingHeardstepsReactiontime,
            Substate::SeekingHeardsteps,
        ] {
            assert_eq!(
                engine.execute_ai_wondering_event(
                    &sim,
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventTimer)
                ),
                Some(false)
            );
            assert_eq!(engine.seek_enemy(owner).base.current_substate, expected);
        }
        assert_eq!(
            engine.seek_enemy(owner).base.last_goto_destination,
            destination
        );
        assert!(
            engine
                .seek_enemy(owner)
                .base
                .last_goto_flags
                .contains(GotoFlags::RUN)
        );
    }

    #[test]
    fn heardsteps_arrival_searches_current_position_and_preserves_noise_location() {
        let (mut engine, assets, owner, target) = fixture(false);
        let here = engine.live_ai_position(owner);
        let remembered = engine.live_ai_position(target);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingHeardsteps;
        ai.base.seek_position = remembered;
        assert_eq!(
            engine.execute_ai_wondering_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint)
            ),
            Some(false)
        );
        let ai = engine.seek_enemy(owner);
        assert_eq!(ai.seek_center, here);
        assert_eq!(
            ai.seek_flags,
            SeekFlags::LOCATION_FIRST | SeekFlags::WALKING
        );
        assert_eq!(ai.base.seek_position, remembered);
        assert!(
            ai.personal_seek_point_1
                .as_ref()
                .is_some_and(|point| point.position == here)
        );
    }

    #[test]
    fn just_watching_sweep_waits_for_completion_without_an_extra_timer() {
        let (mut engine, assets, owner, _) = fixture(false);
        let ai = engine.seek_enemy_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingJustWatching;
        ai.base.launch_timer(60, 40);
        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            assert_eq!(
                engine.execute_ai_wondering_event(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    &Stimulus::new(StimulusType::EventTimer)
                ),
                Some(false)
            );
        });
        let ai = engine.seek_enemy(owner);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingJustWatchingSidewards
        );
        assert!(!ai.base.timer_is_running);
        assert_eq!(draws, vec![crate::sim_rng::RngSite::EnemySeekLook]);
    }

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
