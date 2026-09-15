//! Hearing and reported threats execute their statements against live actors.
use super::*;
use crate::ai::{
    AiState, EmoticonType, Hint, Noise, NoiseType, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::ProfileRank;
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests;

impl EngineInner {
    fn alert_is_forest_merry_man(&self, owner: EntityId) -> bool {
        let entity = self.expect_entity(owner, "alert forest owner");
        self.is_player_aligned_camp(entity.camp())
            && self.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
    }
    fn alert_transient_question(&mut self, owner: EntityId) {
        let frame = self.control.frame_counter;
        self.observation_ai_mut(owner).base.set_transient_emoticon(
            EmoticonType::QuestionMark,
            10,
            frame,
        );
    }
    fn alert_focus_point(&mut self, assets: &LevelAssets, owner: EntityId, position: Position) {
        self.execute_ai_focus_point(assets, owner, position);
    }
    fn alert_face_seek_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let position = self.observation_ai(owner).base.seek_position;
        self.duty_face_position_ground(sim, assets, owner, position);
    }
    pub(super) fn alert_face_noise_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        noise: &Noise,
    ) {
        let position = self.observation_ai(owner).base.seek_position;
        if position != noise.origin.legacy_position()
            || noise.origin.layer.is_none()
            || noise
                .origin
                .sector
                .is_some_and(|sector| sector.arena_index().is_some())
        {
            self.duty_face_position_ground(sim, assets, owner, position);
        } else {
            // A sectorless normalized noise carries its elevation explicitly.
            self.duty_face_position_at_elevation(
                sim,
                assets,
                owner,
                position,
                f32::from(noise.elevation),
            );
        }
    }
    fn alert_noise_is_already_investigated(&self, owner: EntityId) -> bool {
        let state = self.observation_ai(owner).base.current_substate;
        matches!(
            state,
            Substate::SeekingHeardstepsPreReactiontime | Substate::SeekingHeardstepsReactiontime
        ) || state.is_take_money()
            || state.is_fight_for_money()
    }
    pub(super) fn execute_ai_heard_noise(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        noise: &Noise,
    ) {
        let ai = self.observation_ai_mut(owner);
        if i32::from(ai.base.blood_alcohol) > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !ai.has_the_new_task_priority()
        {
            return;
        }
        ai.current_task_priority = ai.new_task_priority;
        ai.investigating_distraction = false;
        if let Some(object) = ai.base.object_of_desire.take() {
            ai.base.forgotten_objects.push(object.get());
        }
        let origin = noise.origin.legacy_position();
        match noise.noise_type {
            NoiseType::Distraction => {
                if self.alert_noise_is_already_investigated(owner) {
                    return;
                }
                let was_default = self.observation_ai(owner).base.current_state == AiState::Default;
                self.observation_stop(sim, assets, owner);
                let ai = self.observation_ai_mut(owner);
                ai.base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                ai.base.seek_position = origin;
                ai.investigating_distraction = true;
                self.observation_say(sim, assets, owner, Remark::HearsNoise);
                self.observation_emoticon(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Seeking,
                    Substate::SeekingHeardstepsPreReactiontime,
                );
                if was_default {
                    self.execute_ai_react_live(
                        sim,
                        assets,
                        owner,
                        crate::parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16,
                    );
                } else {
                    self.observation_timer(owner, 1);
                }
            }
            NoiseType::Pfiiit => {
                let entity = self.expect_entity(owner, "whistle observer");
                let look = entity.element_data().active
                    && !self.entity_data_in_building_sector(entity.element_data())
                    && self
                        .observation_ai(owner)
                        .profile(&assets.profile_manager)
                        .whistle
                        > 0;
                if !look {
                    self.observation_emoticon(owner);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingJustWatching,
                    );
                    self.observation_ai_mut(owner).base.seek_position = origin;
                    self.observation_stop(sim, assets, owner);
                    if self.observation_ai(owner).base.current_state != AiState::Sleeping {
                        self.alert_face_noise_position(sim, assets, owner, noise);
                    }
                    self.observation_say(sim, assets, owner, Remark::HearsNoise);
                    self.observation_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
                    return;
                }
                self.observation_stop(sim, assets, owner);
                self.observation_ai_mut(owner)
                    .base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                let ai = self.observation_ai(owner);
                if ai.base.current_state == AiState::Seeking
                    && ai.get_rank(&assets.profile_manager) != ProfileRank::Officer
                {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingHeardstepsReactiontime,
                    );
                    self.observation_ai_mut(owner).base.seek_position = origin;
                    self.observation_say(sim, assets, owner, Remark::HearsNoise);
                    self.alert_face_noise_position(sim, assets, owner, noise);
                    self.observation_timer(owner, 1);
                } else {
                    self.observation_emoticon(owner);
                    self.observation_say(sim, assets, owner, Remark::HearsNoise);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingHeardWhistling,
                    );
                    self.observation_ai_mut(owner).base.seek_position = origin;
                    self.execute_ai_react_live(
                        sim,
                        assets,
                        owner,
                        crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16,
                    );
                }
            }
            NoiseType::Heeelp | NoiseType::TapTapTap | NoiseType::Aaargh | NoiseType::ZingZing => {
                if noise.noise_type == NoiseType::Heeelp
                    && (self
                        .expect_entity(owner, "help hearing owner")
                        .soldier_data()
                        .is_some_and(|soldier| soldier.rider)
                        || self.observation_ai(owner).base.current_substate
                            == Substate::SeekingJustWatching)
                {
                    return;
                }
                if self.alert_noise_is_already_investigated(owner) {
                    return;
                }
                self.observation_stop(sim, assets, owner);
                self.observation_ai_mut(owner)
                    .base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                let ai = self.observation_ai(owner);
                if ai.base.current_state == AiState::Seeking
                    && ai.base.current_substate != Substate::SeekingGotStopEvent
                    && ai.get_rank(&assets.profile_manager) != ProfileRank::Officer
                {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingHeardstepsReactiontime,
                    );
                    if noise.noise_type != NoiseType::Aaargh {
                        self.observation_say(sim, assets, owner, Remark::HearsNoise);
                    }
                    self.observation_ai_mut(owner).base.seek_position = origin;
                    self.alert_face_noise_position(sim, assets, owner, noise);
                    self.observation_timer(owner, 1);
                } else {
                    if noise.noise_type != NoiseType::Aaargh {
                        self.observation_say(sim, assets, owner, Remark::HearsNoise);
                    }
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingHeardstepsPreReactiontime,
                    );
                    self.observation_emoticon(owner);
                    self.observation_ai_mut(owner).base.seek_position = origin;
                    if noise.noise_type != NoiseType::Aaargh
                        && self.observation_ai(owner).base.current_state == AiState::Default
                    {
                        self.execute_ai_react_live(
                            sim,
                            assets,
                            owner,
                            crate::parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16,
                        );
                    } else {
                        self.observation_timer(owner, 1);
                    }
                }
            }
            NoiseType::Bonk | NoiseType::Zonk | NoiseType::Pling
                if self.observation_ai(owner).base.current_state == AiState::Default =>
            {
                self.observation_stop(sim, assets, owner);
                if noise.noise_type == NoiseType::Zonk {
                    self.observation_say(sim, assets, owner, Remark::Arrow);
                }
                self.observation_emoticon(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingWatching,
                );
                self.observation_ai_mut(owner).base.seek_position = origin;
                self.alert_face_noise_position(sim, assets, owner, noise);
                self.observation_timer(owner, 50);
            }
            NoiseType::Logs | NoiseType::Drawbridge
                if self.observation_ai(owner).base.current_state == AiState::Default =>
            {
                self.observation_stop(sim, assets, owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingWatching,
                );
                self.observation_ai_mut(owner).base.seek_position = origin;
                self.alert_face_noise_position(sim, assets, owner, noise);
                let frames = 70
                    + crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::SoldierNoiseCooldown,
                        0..60,
                    );
                self.observation_timer(owner, frames);
            }
            _ => {}
        }
    }
    pub(super) fn execute_ai_look_there_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        if !self.alert_is_forest_merry_man(owner) {
            self.alert_transient_question(owner);
        }
        self.observation_stop(sim, assets, owner);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingWatching,
        );
        self.observation_ai_mut(owner).base.seek_position = position;
        self.alert_focus_point(assets, owner, position);
        self.alert_face_seek_position(sim, assets, owner);
        self.observation_timer(owner, 100);
    }
    pub(super) fn execute_ai_tower_alert_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        hint: &Hint,
    ) {
        self.alert_transient_question(owner);
        self.observation_ai_mut(owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);
        if self.observation_ai(owner).get_rank(&assets.profile_manager) == ProfileRank::Knight {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingKnightWatchingTowerGuard,
            );
        } else {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingWatchingTowerGuard,
            );
        }
        self.observation_ai_mut(owner).base.seek_position = hint.seek_point;
        let position = self.observation_ai(owner).base.seek_position;
        self.alert_focus_point(assets, owner, position);
        let teller =
            self.expect_human_id_for_ai_handle(hint.who_tells_me.get(), "tower alert caller");
        self.observation_face_entity(sim, assets, owner, teller, false);
        self.observation_timer(owner, 100);
    }
    pub(super) fn execute_ai_tower_call_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        hint: &Hint,
    ) {
        let ai = self.observation_ai_mut(owner);
        ai.base.seek_position = hint.seek_point;
        ai.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);
        match ai.get_rank(&assets.profile_manager) {
            ProfileRank::Soldier => self.execute_ai_alert_officer_for_caller(
                sim,
                assets,
                owner,
                crate::ai::OfficerAlertCaller::TowerGuardCalled,
            ),
            ProfileRank::Officer => {
                let center = self.observation_ai(owner).base.seek_position;
                self.execute_ai_alert_soldiers(sim, assets, owner, center, 0);
            }
            ProfileRank::Knight => panic!("knight cannot receive tower guard call"),
            ProfileRank::None => {}
        }
    }
    pub(super) fn execute_ai_combat_alert_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        self.alert_transient_question(owner);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Seeking,
            Substate::SeekingCombatAlertReactiontime,
        );
        self.observation_ai_mut(owner).base.seek_position = position;
        self.alert_focus_point(assets, owner, position);
        self.alert_face_seek_position(sim, assets, owner);
        self.execute_ai_react_live(
            sim,
            assets,
            owner,
            crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16,
        );
    }
}
