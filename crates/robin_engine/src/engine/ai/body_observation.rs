//! Body sightings and reaction timers run against live actors between callbacks.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, EmoticonType, HumanHandle, Remark, ReportType, Substate};
use crate::element::Human as _;
use crate::profiles::ProfileRank;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(super) fn execute_ai_seen_body(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        body: HumanHandle,
    ) {
        let body_id = self.expect_human_id_for_ai_handle(body, "seen body");
        assert_ne!(body_id, owner, "body observer cannot observe itself");
        let ai = self.seek_enemy(owner);
        let is_charly = ai.base.current_state == AiState::Seeking
            && ai.base.my_reconnaissance_report.report_type == ReportType::MissedCharly
            && ai.base.my_reconnaissance_report.charly == Some(AiEntityHandle::new(body));
        let body_position = self.live_ai_position(body_id);
        let target = self.expect_entity(body_id, "seen body report");
        let dead_npc = target.is_dead() && target.is_npc();
        let ai = self.seek_enemy_mut(owner);
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
                    self.unalert_body_charly_seekers(sim, assets, owner, body);
                }
                self.execute_seek_body(sim, assets, owner, body_id);
                return;
            }
            _ => {}
        }
        let stuck = self
            .expect_entity(body_id, "body remark target")
            .human_data()
            .expect("body must remain human")
            .stuck_under_nets_counter
            > 0;
        self.execute_ai_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt {
                remark: if stuck {
                    Remark::SeesFriendUnderNet
                } else {
                    Remark::SeesBody
                },
                flags: 0,
            },
        );
        let hint = self.live_ai_position(body_id);
        self.execute_ai_look_there(sim, assets, owner, hint, 100);
        self.seek_enemy_mut(owner).seen_dead_body = false;
        self.stop_ai_owner(sim, assets, owner);
        let body_position = self.live_ai_position(body_id);
        let ai = self.seek_enemy_mut(owner);
        ai.base.seek_position = body_position;
        ai.base.detected_body = Some(AiEntityHandle::new(body));
        self.execute_ai_focus(owner, Some(AiEntityHandle::new(body)));

        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Seeking,
            Substate::SeekingBodyReactiontime,
        );
        // A state callback can replace the remembered point before Face reads it.
        let position = self.seek_enemy(owner).base.seek_position;
        let target = self.position_to_point_3d(
            assets,
            position.sector,
            position.level,
            position.x,
            position.y,
        );
        let here = self
            .expect_entity(owner, "body facing owner")
            .element_data()
            .position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - here.x,
            target.y - here.y,
        );
        self.duty_face_direction(sim, assets, owner, direction as u16);
        self.seek_enemy_mut(owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);

        if is_charly {
            self.unalert_body_charly_seekers(sim, assets, owner, body);
        }
        self.react_to_seen_body(sim, assets, owner);
    }

    fn unalert_body_charly_seekers(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        body: HumanHandle,
    ) {
        let body = self.expect_human_id_for_ai_handle(body, "body checkpoint seekers");
        self.unalert_live_charly_seekers(sim, assets, owner, body);
    }

    fn react_to_seen_body(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let entity = self.expect_entity(owner, "body reaction owner");
        let frames = if self.is_player_aligned_camp(entity.camp())
            && self.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
        {
            3
        } else {
            let difficulty = self.control.sim_config.difficulty;
            let modifier = if self.is_hostile_to_player_camp(entity.camp()) {
                if difficulty == crate::player_profile::DifficultyLevel::Hard
                    && !sim.config().fix_hard_reaction_times
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
                    .seek_enemy(owner)
                    .profile(&assets.profile_manager)
                    .intelligence as f32)
                * 0.01
                * crate::parameters_ai::AI_MAX_DEADBODY_REACTIONTIME as f32
                * modifier
                + 1.0) as u32
        };
        let frame = self.control.frame_counter;
        self.seek_enemy_mut(owner).base.launch_timer(frames, frame);
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

    pub(super) fn execute_ai_body_reaction_timer(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let body = self
            .seek_enemy(owner)
            .base
            .detected_body
            .expect("body reaction requires detected body");
        let body_id = self.expect_human_id_for_ai_handle(body.get(), "body reaction target");
        let rank = self.seek_enemy(owner).profile(&assets.profile_manager).rank;
        let mut officer = None;
        let mut delegate = false;
        match rank {
            ProfileRank::Soldier => {
                officer = self.near_officer_informed_about_body(assets, owner, body_id)
            }
            ProfileRank::Officer => {
                let here = self
                    .expect_entity(owner, "body distance owner")
                    .element_data()
                    .position();
                let target = self
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
                    let ai = self.seek_enemy(owner);
                    let entity = self.expect_entity(owner, "body delegation owner");
                    delegate = i32::from(ai.base.blood_alcohol)
                        <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                        && entity.element_data().active
                        && !self.entity_data_in_building_sector(entity.element_data())
                        && (ai.profile(&assets.profile_manager).initiative < 50
                            || !ai.base.patrol.is_empty());
                }
            }
            ProfileRank::Knight | ProfileRank::None => {}
        }
        if let Some(officer) = officer {
            let position = self.live_ai_position(officer);
            let elevation = self
                .expect_entity(officer, "body advice facing officer")
                .position_iface()
                .get_elevation();
            self.duty_face_position_at_elevation(sim, assets, owner, position, elevation);
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

            let frame = self.control.frame_counter;
            self.seek_enemy_mut(owner).base.launch_timer(100, frame);
        } else if delegate {
            self.execute_ai_officer_look_for_soldier(sim, assets, owner, ReportType::Body);
        } else {
            let body = self
                .seek_enemy(owner)
                .base
                .detected_body
                .expect("body examination requires detected body");
            let body_id = self.expect_human_id_for_ai_handle(body.get(), "body examination target");
            self.execute_seek_body(sim, assets, owner, body_id);
        }
    }
}
