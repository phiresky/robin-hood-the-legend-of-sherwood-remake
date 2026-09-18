//! Hearing and reported threats execute their statements against live actors.
use super::*;
use crate::ai::{
    AiState, EmoticonType, Hint, Noise, NoiseType, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::ProfileRank;
#[cfg(test)]
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
    #[cfg(test)]
    pub(super) fn alert_face_noise_position(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        noise: &Noise,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).alert_face_noise_position(noise)
    }

    fn alert_noise_is_already_investigated(&self, owner: EntityId) -> bool {
        let state = self.observation_ai(owner).base.current_substate;
        matches!(
            state,
            Substate::SeekingHeardstepsPreReactiontime | Substate::SeekingHeardstepsReactiontime
        ) || state.is_take_money()
            || state.is_fight_for_money()
    }
    #[cfg(test)]
    pub(super) fn execute_ai_heard_noise(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        noise: &Noise,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_heard_noise(noise)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_look_there_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_look_there_reaction(position)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_tower_alert_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        hint: &Hint,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_tower_alert_reaction(hint)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_combat_alert_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_combat_alert_reaction(position)
    }
}

impl AiOwnerCtx<'_> {
    fn alert_face_seek_position(&mut self) {
        let position = self.engine.observation_ai(self.owner).base.seek_position;
        self.duty_face_position_ground(position);
    }

    pub(super) fn alert_face_noise_position(&mut self, noise: &Noise) {
        let position = self.engine.observation_ai(self.owner).base.seek_position;
        if position != noise.origin.legacy_position()
            || noise.origin.layer.is_none()
            || noise
                .origin
                .sector
                .is_some_and(|sector| sector.arena_index().is_some())
        {
            self.duty_face_position_ground(position);
        } else {
            // A sectorless normalized noise carries its elevation explicitly.
            self.duty_face_position_at_elevation(position, f32::from(noise.elevation));
        }
    }

    pub(super) fn execute_ai_heard_noise(&mut self, noise: &Noise) {
        let ai = self.engine.observation_ai_mut(self.owner);
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
                if self.engine.alert_noise_is_already_investigated(self.owner) {
                    return;
                }
                let was_default =
                    self.engine.observation_ai(self.owner).base.current_state == AiState::Default;
                self.observation_stop();
                let ai = self.engine.observation_ai_mut(self.owner);
                ai.base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                ai.base.seek_position = origin;
                ai.investigating_distraction = true;
                self.observation_say(Remark::HearsNoise);
                self.engine.observation_emoticon(self.owner);
                self.duty_set_state(AiState::Seeking, Substate::SeekingHeardstepsPreReactiontime);
                if was_default {
                    self.execute_ai_react_live(
                        crate::parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16,
                    );
                } else {
                    self.engine.observation_timer(self.owner, 1);
                }
            }
            NoiseType::Pfiiit => {
                let entity = self.engine.expect_entity(self.owner, "whistle observer");
                let look = entity.element_data().active
                    && !self
                        .engine
                        .entity_data_in_building_sector(entity.element_data())
                    && self
                        .engine
                        .observation_ai(self.owner)
                        .profile(&self.assets.profile_manager)
                        .whistle
                        > 0;
                if !look {
                    self.engine.observation_emoticon(self.owner);
                    self.duty_set_state(AiState::Seeking, Substate::SeekingJustWatching);
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .seek_position = origin;
                    self.observation_stop();
                    if self.engine.observation_ai(self.owner).base.current_state
                        != AiState::Sleeping
                    {
                        self.alert_face_noise_position(noise);
                    }
                    self.observation_say(Remark::HearsNoise);
                    self.engine.observation_timer(
                        self.owner,
                        crate::parameters_ai::AI_FIRST_LOOK_TIME as u32,
                    );
                    return;
                }
                self.observation_stop();
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                let ai = self.engine.observation_ai(self.owner);
                if ai.base.current_state == AiState::Seeking
                    && ai.get_rank(&self.assets.profile_manager) != ProfileRank::Officer
                {
                    self.duty_set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .seek_position = origin;
                    self.observation_say(Remark::HearsNoise);
                    self.alert_face_noise_position(noise);
                    self.engine.observation_timer(self.owner, 1);
                } else {
                    self.engine.observation_emoticon(self.owner);
                    self.observation_say(Remark::HearsNoise);
                    self.duty_set_state(AiState::Wondering, Substate::WonderingHeardWhistling);
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .seek_position = origin;
                    self.execute_ai_react_live(
                        crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16,
                    );
                }
            }
            NoiseType::Heeelp | NoiseType::TapTapTap | NoiseType::Aaargh | NoiseType::ZingZing => {
                if noise.noise_type == NoiseType::Heeelp
                    && (self
                        .engine
                        .expect_entity(self.owner, "help hearing owner")
                        .soldier_data()
                        .is_some_and(|soldier| soldier.rider)
                        || self.engine.observation_ai(self.owner).base.current_substate
                            == Substate::SeekingJustWatching)
                {
                    return;
                }
                if self.engine.alert_noise_is_already_investigated(self.owner) {
                    return;
                }
                self.observation_stop();
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .my_reconnaissance_report
                    .update(ReportType::Noise, origin);
                let ai = self.engine.observation_ai(self.owner);
                if ai.base.current_state == AiState::Seeking
                    && ai.base.current_substate != Substate::SeekingGotStopEvent
                    && ai.get_rank(&self.assets.profile_manager) != ProfileRank::Officer
                {
                    self.duty_set_state(AiState::Seeking, Substate::SeekingHeardstepsReactiontime);
                    if noise.noise_type != NoiseType::Aaargh {
                        self.observation_say(Remark::HearsNoise);
                    }
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .seek_position = origin;
                    self.alert_face_noise_position(noise);
                    self.engine.observation_timer(self.owner, 1);
                } else {
                    if noise.noise_type != NoiseType::Aaargh {
                        self.observation_say(Remark::HearsNoise);
                    }
                    self.duty_set_state(
                        AiState::Seeking,
                        Substate::SeekingHeardstepsPreReactiontime,
                    );
                    self.engine.observation_emoticon(self.owner);
                    self.engine
                        .observation_ai_mut(self.owner)
                        .base
                        .seek_position = origin;
                    if noise.noise_type != NoiseType::Aaargh
                        && self.engine.observation_ai(self.owner).base.current_state
                            == AiState::Default
                    {
                        self.execute_ai_react_live(
                            crate::parameters_ai::AI_MAX_STEPS_REACTIONTIME as u16,
                        );
                    } else {
                        self.engine.observation_timer(self.owner, 1);
                    }
                }
            }
            NoiseType::Bonk | NoiseType::Zonk | NoiseType::Pling
                if self.engine.observation_ai(self.owner).base.current_state
                    == AiState::Default =>
            {
                self.observation_stop();
                if noise.noise_type == NoiseType::Zonk {
                    self.observation_say(Remark::Arrow);
                }
                self.engine.observation_emoticon(self.owner);
                self.duty_set_state(AiState::Wondering, Substate::WonderingWatching);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .seek_position = origin;
                self.alert_face_noise_position(noise);
                self.engine.observation_timer(self.owner, 50);
            }
            NoiseType::Logs | NoiseType::Drawbridge
                if self.engine.observation_ai(self.owner).base.current_state
                    == AiState::Default =>
            {
                self.observation_stop();
                self.duty_set_state(AiState::Wondering, Substate::WonderingWatching);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .seek_position = origin;
                self.alert_face_noise_position(noise);
                let frames = 70
                    + crate::sim_rng::u32(
                        self.sim,
                        crate::sim_rng::RngSite::SoldierNoiseCooldown,
                        0..60,
                    );
                self.engine.observation_timer(self.owner, frames);
            }
            _ => {}
        }
    }

    pub(super) fn execute_ai_look_there_reaction(&mut self, position: Position) {
        if !self.engine.alert_is_forest_merry_man(self.owner) {
            self.engine.alert_transient_question(self.owner);
        }
        self.observation_stop();
        self.duty_set_state(AiState::Wondering, Substate::WonderingWatching);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .seek_position = position;
        self.engine
            .alert_focus_point(self.assets, self.owner, position);
        self.alert_face_seek_position();
        self.engine.observation_timer(self.owner, 100);
    }

    pub(super) fn execute_ai_tower_alert_reaction(&mut self, hint: &Hint) {
        self.engine.alert_transient_question(self.owner);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);
        if self
            .engine
            .observation_ai(self.owner)
            .get_rank(&self.assets.profile_manager)
            == ProfileRank::Knight
        {
            self.duty_set_state(AiState::Seeking, Substate::SeekingKnightWatchingTowerGuard);
        } else {
            self.duty_set_state(AiState::Wondering, Substate::WonderingWatchingTowerGuard);
        }
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .seek_position = hint.seek_point;
        let position = self.engine.observation_ai(self.owner).base.seek_position;
        self.engine
            .alert_focus_point(self.assets, self.owner, position);
        let teller = self
            .engine
            .expect_human_id_for_ai_handle(hint.who_tells_me.get(), "tower alert caller");
        self.observation_face_entity(teller, false);
        self.engine.observation_timer(self.owner, 100);
    }

    pub(super) fn execute_ai_tower_call_reaction(&mut self, hint: &Hint) {
        let ai = self.engine.observation_ai_mut(self.owner);
        ai.base.seek_position = hint.seek_point;
        ai.base
            .my_reconnaissance_report
            .update(ReportType::Enemy, hint.seek_point);
        match ai.get_rank(&self.assets.profile_manager) {
            ProfileRank::Soldier => self.execute_ai_alert_officer_for_caller(
                crate::ai::OfficerAlertCaller::TowerGuardCalled,
            ),
            ProfileRank::Officer => {
                let center = self.engine.observation_ai(self.owner).base.seek_position;
                self.execute_ai_alert_soldiers(center, 0);
            }
            ProfileRank::Knight => panic!("knight cannot receive tower guard call"),
            ProfileRank::None => {}
        }
    }

    pub(super) fn execute_ai_combat_alert_reaction(&mut self, position: Position) {
        self.engine.alert_transient_question(self.owner);
        self.duty_set_state(AiState::Seeking, Substate::SeekingCombatAlertReactiontime);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .seek_position = position;
        self.engine
            .alert_focus_point(self.assets, self.owner, position);
        self.alert_face_seek_position();
        self.execute_ai_react_live(crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16);
    }
}
