//! Body sightings and reaction timers run against live actors between callbacks.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, EmoticonType, HumanHandle, Remark, ReportType, Substate};
use crate::element::Human as _;
use crate::engine::TickCtx;
use crate::profiles::ProfileRank;
#[cfg(test)]
use crate::sim_rng::SimulationContext;

impl EngineInner {
    #[cfg(test)]
    pub(super) fn execute_ai_seen_body(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        body: HumanHandle,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_seen_body(body)
    }

    fn near_officer_informed_about_body(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        body: EntityId,
    ) -> Option<EntityId> {
        let viewer = self.expect_entity(owner, "body advice viewer");
        let camp = viewer.camp();
        for &handle in self.world.soldier_registry.all().iter() {
            let id = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            let Entity::Soldier(soldier) = self.expect_entity(id, "body advice officer") else {
                unreachable!()
            };
            if !self.camps_are_allied(soldier.soldier.cached_camp, camp) {
                continue;
            }
            let ai = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("body advice soldier requires enemy AI");
            if ai.profile(&assets.profile_manager).rank != ProfileRank::Officer
                || !soldier.is_able_to_fight()
            {
                continue;
            }
            // The visibility ray precedes the lock and detectable-list gates.
            if !viewer.element_data().active || !soldier.element.active {
                continue;
            }
            if !patrol_member_visible_from_raw_world(
                viewer.element_data().position(),
                viewer.soldier_data().is_some_and(|soldier| soldier.rider),
                viewer
                    .ai_actor_data()
                    .expect("body advice viewer requires NPC data")
                    .view_radius,
                self.entity_data_in_building_sector(viewer.element_data()),
                soldier.element.position(),
                soldier.element.posture(),
                soldier.soldier.rider,
                soldier.element.direction(),
                self.entity_data_in_building_sector(&soldier.element),
                self.sight_obstacles(assets),
            ) {
                continue;
            }
            if ai.base.script_locked {
                continue;
            }
            if soldier.npc.detectable_lists[crate::element::DetectableType::Body as usize]
                .iter()
                .any(|entry| entry.element == Some(body))
            {
                continue;
            }
            return Some(id);
        }
        None
    }

    #[cfg(test)]
    pub(super) fn execute_ai_body_reaction_timer(&mut self, tcx: TickCtx<'_>, owner: EntityId) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_body_reaction_timer()
    }
}

impl AiOwnerCtx<'_> {
    pub(super) fn execute_ai_seen_body(&mut self, body: HumanHandle) {
        let body_id = self.engine.expect_human_id_for_ai_handle(body, "seen body");
        // Net publication includes the victim in its own detectable list.
        // A later self-sighting still updates the report and follows the normal
        // priority and examination path, even after the net has gone away.
        let ai = self.engine.seek_enemy(self.owner);
        let is_charly = ai.base.current_state == AiState::Seeking
            && ai.base.my_reconnaissance_report.report_type == ReportType::MissedCharly
            && ai.base.my_reconnaissance_report.charly == Some(AiEntityHandle::new(body));
        let body_position = self.engine.live_ai_position(body_id);
        let target = self.engine.expect_entity(body_id, "seen body report");
        let dead_npc = target.is_dead() && target.is_npc();
        let ai = self.engine.seek_enemy_mut(self.owner);
        ai.base.my_reconnaissance_report.add_seen_body(body);
        ai.base
            .my_reconnaissance_report
            .update(ReportType::Body, body_position);
        if dead_npc {
            ai.base.missed_in_action.push(body);
        }
        if i32::from(ai.base.blood_alcohol) > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !ai.has_the_new_task_priority()
        {
            return;
        }
        ai.current_task_priority = ai.new_task_priority;
        if let Some(object) = ai.base.object_of_desire.take() {
            ai.base.forgotten_objects.push(object.get());
        }
        match ai.base.current_substate {
            Substate::SeekingBodyReactiontime
            | Substate::SeekingBody
            | Substate::SeekingNet
            | Substate::SeekingBodyLookingDeadBody
            | Substate::SeekingBodyAwakeningSleeperr => {
                if ai.base.detected_body != Some(AiEntityHandle::new(body)) {
                    ai.other_bodies_to_examine.push(body);
                }
                return;
            }
            Substate::SeekingSeekpoint
            | Substate::SeekingCharly
            | Substate::SeekingSeekpointPassedAmbushPointLeft
            | Substate::SeekingSeekpointPassedAmbushPointRight
            | Substate::SeekingSeekpointCheckingAmbushPoint => {
                if is_charly {
                    self.unalert_body_charly_seekers(body);
                }
                self.execute_seek_body(body_id);
                return;
            }
            _ => {}
        }
        let stuck = self
            .engine
            .expect_entity(body_id, "body remark target")
            .human_data()
            .expect("body must remain human")
            .stuck_under_nets_counter
            > 0;
        self.execute_ai_speech(crate::ai::AiSpeechAttempt {
            remark: if stuck {
                Remark::SeesFriendUnderNet
            } else {
                Remark::SeesBody
            },
            flags: 0,
        });
        let hint = self.engine.live_ai_position(body_id);
        self.engine.execute_ai_look_there(
            TickCtx::new(self.sim, self.assets),
            self.owner,
            hint,
            100,
        );
        self.engine.seek_enemy_mut(self.owner).seen_dead_body = false;
        self.stop_ai_owner();
        let body_position = self.engine.live_ai_position(body_id);
        let ai = self.engine.seek_enemy_mut(self.owner);
        ai.base.seek_position = body_position;
        ai.base.detected_body = Some(AiEntityHandle::new(body));
        self.engine
            .execute_ai_focus(self.owner, Some(AiEntityHandle::new(body)));

        self.duty_set_state(AiState::Seeking, Substate::SeekingBodyReactiontime);
        // A state callback can replace the remembered point before Face reads it.
        let position = self.engine.seek_enemy(self.owner).base.seek_position;
        let target = self.engine.position_to_point_3d(
            self.assets,
            position.sector,
            position.level,
            position.x,
            position.y,
        );
        let here = self
            .engine
            .expect_entity(self.owner, "body facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - here.x,
            target.y - here.y,
        );
        self.duty_face_direction(direction as u16);
        self.engine
            .seek_enemy_mut(self.owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);

        if is_charly {
            self.unalert_body_charly_seekers(body);
        }
        self.react_to_seen_body();
    }

    fn unalert_body_charly_seekers(&mut self, body: HumanHandle) {
        let body = self
            .engine
            .expect_human_id_for_ai_handle(body, "body checkpoint seekers");
        self.unalert_live_charly_seekers(body);
    }

    fn react_to_seen_body(&mut self) {
        let entity = self.engine.expect_entity(self.owner, "body reaction owner");
        let frames = if self.engine.is_player_aligned_camp(entity.camp())
            && self.engine.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
        {
            3
        } else {
            let difficulty = self.engine.control.sim_config.difficulty;
            let modifier = if self.engine.is_hostile_to_player_camp(entity.camp()) {
                if difficulty == crate::player_profile::DifficultyLevel::Hard
                    && !self.sim.config().fix_hard_reaction_times
                {
                    2.0
                } else {
                    crate::player_profile::DifficultyRules::percent_as_f32(
                        difficulty.rules().reaction_time_percent,
                    )
                }
            } else {
                1.0
            };
            ((100.0
                - self
                    .engine
                    .seek_enemy(self.owner)
                    .profile(&self.assets.profile_manager)
                    .intelligence as f32)
                * 0.01
                * crate::parameters_ai::AI_MAX_DEADBODY_REACTIONTIME as f32
                * modifier
                + 1.0) as u32
        };
        let frame = self.engine.control.frame_counter;
        self.engine
            .seek_enemy_mut(self.owner)
            .base
            .launch_timer(frames, frame);
    }

    pub(super) fn execute_ai_body_reaction_timer(&mut self) {
        let body = self
            .engine
            .seek_enemy(self.owner)
            .base
            .detected_body
            .expect("body reaction requires detected body");
        let body_id = self
            .engine
            .expect_human_id_for_ai_handle(body.get(), "body reaction target");
        let rank = self
            .engine
            .seek_enemy(self.owner)
            .profile(&self.assets.profile_manager)
            .rank;
        let mut officer = None;
        let mut delegate = false;
        match rank {
            ProfileRank::Soldier => {
                officer =
                    self.engine
                        .near_officer_informed_about_body(self.assets, self.owner, body_id)
            }
            ProfileRank::Officer => {
                let here = self
                    .engine
                    .expect_entity(self.owner, "body distance owner")
                    .element_data()
                    .position();
                let target = self
                    .engine
                    .expect_entity(body_id, "body distance target")
                    .element_data()
                    .position();
                let distance = (target.x - here.x)
                    .abs()
                    .max(
                        ((target.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                            .abs(),
                    )
                    .max((target.z - here.z).abs());
                if distance > crate::ai_enemy::combat::OFFICER_EXAMINE_BODY_HIMSELF_DISTANCE as f32
                {
                    let ai = self.engine.seek_enemy(self.owner);
                    let entity = self
                        .engine
                        .expect_entity(self.owner, "body delegation owner");
                    delegate = i32::from(ai.base.blood_alcohol)
                        <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                        && entity.element_data().active
                        && !self
                            .engine
                            .entity_data_in_building_sector(entity.element_data())
                        && (ai.profile(&self.assets.profile_manager).initiative < 50
                            || !ai.base.patrol.is_empty());
                }
            }
            ProfileRank::Knight | ProfileRank::None => {}
        }
        if let Some(officer) = officer {
            let position = self.engine.live_ai_position(officer);
            let elevation = self
                .engine
                .expect_entity(officer, "body advice facing officer")
                .position_iface()
                .get_elevation();
            self.duty_face_position_at_elevation(position, elevation);
            self.duty_set_state(AiState::Default, Substate::DefaultLookingOfficerForAdvice);
            self.engine
                .seek_enemy_mut(self.owner)
                .base
                .set_emoticon(EmoticonType::QuestionMark);

            let frame = self.engine.control.frame_counter;
            self.engine
                .seek_enemy_mut(self.owner)
                .base
                .launch_timer(100, frame);
        } else if delegate {
            self.execute_ai_officer_look_for_soldier(ReportType::Body);
        } else {
            let body = self
                .engine
                .seek_enemy(self.owner)
                .base
                .detected_body
                .expect("body examination requires detected body");
            let body_id = self
                .engine
                .expect_human_id_for_ai_handle(body.get(), "body examination target");
            self.execute_seek_body(body_id);
        }
    }
}
