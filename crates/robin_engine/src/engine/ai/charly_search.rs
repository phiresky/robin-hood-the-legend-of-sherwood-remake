//! Checkpoint searches retain their route while querying the current actor state.
use super::*;
use crate::ai::{
    AiState, AlertLevel, DutyFlags, EmoticonType, GotoFlags, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::{SeekFlags, task_priority};
use crate::profiles::ProfileRank;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_search_charly(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_search_charly()
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_search_charly(&mut self) {
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);

        if self
            .engine
            .observation_ai(self.owner)
            .get_rank(&self.assets.profile_manager)
            == ProfileRank::Officer
        {
            self.execute_ai_missed_charly_alert();
            return;
        }
        self.observation_say(Remark::MissesCharly);
        let ai = self.engine.observation_ai_mut(self.owner);
        ai.search_charly_way.clear();
        ai.base.macro_in_progress = false;
        ai.current_task_priority = task_priority::MISSED_FRIEND;
        ai.seeking_charly = true;
        let Some(charly) = ai.base.checkpoint_charly else {
            self.execute_ai_return_to_duty(DutyFlags::empty());
            return;
        };
        let charly = self
            .engine
            .expect_human_id_for_ai_handle(charly.get(), "missing checkpoint");
        let checkpoint = self.engine.ai(charly, "checkpoint search path");
        if checkpoint.has_patrol_path {
            let path = checkpoint
                .patrol_path
                .as_ref()
                .map(|p| p.hiking_path_index)
                .or(checkpoint.detached_patrol_path_status.hiking_path_index)
                .expect("checkpoint path must resolve")
                .get() as usize;
            let here = self.engine.live_ai_position(self.owner);
            let points = &self.assets.navigation.hiking_paths[path].waypoints;
            let mut best = None;
            let mut distance = 65432.0_f32;
            for (index, point) in points.iter().enumerate() {
                let d = (point.x as f32 - here.x)
                    .abs()
                    .max((point.y as f32 - here.y).abs());
                if d < distance {
                    best = Some(index);
                    distance = d;
                }
            }
            let best = best.expect("checkpoint route requires a waypoint within search range");
            let checkpoint = self.engine.ai_mut(charly, "checkpoint nearest waypoint");
            if let Some(path) = checkpoint.patrol_path.as_mut() {
                path.last_waypoint_index = path.current_waypoint_index;
                path.current_waypoint_index = best as u8;
            } else {
                checkpoint.detached_patrol_path_status.last_waypoint_index = checkpoint
                    .detached_patrol_path_status
                    .current_waypoint_index;
                checkpoint
                    .detached_patrol_path_status
                    .current_waypoint_index = best as u8;
            }
            let next = (best + 1) % points.len();
            let a = &points[best];
            let b = &points[next];
            let dot = (a.x as f32 - here.x) * (b.x as f32 - a.x as f32)
                + (a.y as f32 - here.y) * (b.y as f32 - a.y as f32);
            let start = if dot < 0.0 { next } else { best };
            for offset in 0..points.len() {
                let index = (start + offset) % points.len();
                let point = &points[index];
                self.engine
                    .observation_ai_mut(self.owner)
                    .search_charly_way
                    .push(Position {
                        x: point.x as f32,
                        y: point.y as f32,
                        sector: self.assets.navigation.hiking_waypoint_sector(
                            path,
                            index,
                            point.sector,
                        ),
                        level: point.level,
                    });
            }
        } else {
            let position = checkpoint.initial_position;
            self.engine
                .observation_ai_mut(self.owner)
                .search_charly_way
                .push(position);
        }
        self.duty_set_state(AiState::Seeking, Substate::SeekingCharly);
        self.engine.execute_ai_set_alert_status(
            self.assets,
            self.owner,
            AlertLevel::Yellow,
            crate::ai::AlertFlags::empty(),
        );

        let ai = self.engine.observation_ai(self.owner);
        let position = ai.search_charly_way[0];
        let flags = GotoFlags::RUN
            | if ai.search_charly_way.len() > 1 {
                GotoFlags::DONT_STOP
            } else {
                GotoFlags::empty()
            };
        self.duty_go_to(position, flags);
    }

    pub(in crate::engine) fn execute_ai_missed_charly_alert(&mut self) {
        self.observation_say(Remark::DidntFindCharly);
        let here = self.engine.live_ai_position(self.owner);
        let frame = self.engine.control.frame_counter;
        let ai = self.engine.observation_ai_mut(self.owner);
        ai.base.seek_position = here;
        ai.base
            .my_reconnaissance_report
            .update(ReportType::MissedCharly, here);
        ai.base.my_reconnaissance_report.charly = ai.base.checkpoint_charly;
        ai.base.frame_when_enemy_detected = frame;
        let charly = ai
            .base
            .checkpoint_charly
            .expect("missed checkpoint alert requires checkpoint");
        let charly = self
            .engine
            .expect_human_id_for_ai_handle(charly.get(), "missed checkpoint alert");
        self.engine
            .enemy_ai_mut(charly, "checkpoint reporting status")
            .reported_to_officer = false;
        match self
            .engine
            .observation_ai(self.owner)
            .get_rank(&self.assets.profile_manager)
        {
            ProfileRank::Soldier => {
                if self.execute_ai_alert_officer() {
                    return;
                }
            }
            ProfileRank::Officer => {
                let here = self.engine.live_ai_position(self.owner);
                if self.execute_ai_alert_soldiers(here, SeekFlags::CHARLY_SEEK.bits()) {
                    return;
                }
            }
            _ => {}
        }
        let checkpoint = self
            .engine
            .observation_ai(self.owner)
            .base
            .checkpoint_charly
            .expect("failed checkpoint alert requires checkpoint");
        let checkpoint = self
            .engine
            .expect_human_id_for_ai_handle(checkpoint.get(), "failed checkpoint alert");
        let radius = if self
            .engine
            .ai(checkpoint, "failed checkpoint path")
            .has_patrol_path
        {
            crate::parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS
        } else {
            crate::parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS
        };
        let here = self.engine.live_ai_position(self.owner);
        self.execute_ai_seek_area(
            here,
            radius as u16,
            SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
            crate::ai_enemy::UNDEFINED_DIRECTION,
        );
    }
}
