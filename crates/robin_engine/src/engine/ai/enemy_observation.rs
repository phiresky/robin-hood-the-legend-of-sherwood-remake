//! Sight and arrow reactions retain actor identities across synchronous callbacks.
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, EmoticonType, GotoFlags, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::{EnemyAi, ProfileRank, task_priority};
#[cfg(test)]
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests;

impl EngineInner {
    pub(super) fn observation_ai(&self, owner: EntityId) -> &EnemyAi {
        self.enemy_ai(owner, "observation owner")
    }
    pub(super) fn observation_ai_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.enemy_ai_mut(owner, "observation owner")
    }
    pub(super) fn observation_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.observation_ai_mut(owner)
            .base
            .launch_timer(frames, frame);
    }
    fn observation_forget_object(&mut self, owner: EntityId) {
        let ai = self.observation_ai_mut(owner);
        if let Some(object) = ai.base.object_of_desire.take() {
            ai.base.forgotten_objects.push(object.get());
        }
    }
    fn observation_focus(&mut self, owner: EntityId, target: Option<AiEntityHandle>) {
        self.execute_ai_focus(owner, target);
    }
    pub(super) fn observation_emoticon(&mut self, owner: EntityId) {
        self.observation_ai_mut(owner)
            .base
            .set_emoticon(EmoticonType::QuestionMark);
    }
    #[cfg(test)]
    pub(super) fn execute_ai_react_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        maximum: u16,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_react_live(maximum)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_seen_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: u32,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_seen_enemy(target)
    }

    fn observation_enemy_below(&self, owner: EntityId, target: EntityId) -> bool {
        let me = self.expect_entity(owner, "below observer");
        if me.element_data().posture() == crate::element::Posture::LeaningOut {
            return true;
        }
        let a = me.element_data().position();
        let b = self
            .expect_entity(target, "below target")
            .element_data()
            .position();
        let height = b.z - a.z;
        if height >= 0.0 {
            return false;
        }
        if height < -50.0 {
            return true;
        }
        let dx = b.x - a.x;
        let dy = (b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        dx * dx + dy * dy <= height * height
    }
    #[cfg(test)]
    pub(super) fn execute_ai_seen_shadow(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_seen_shadow(position)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_received_arrow(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        origin: Position,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_received_arrow(origin)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_seen_object(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        object: u32,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_seen_object(object)
    }

    fn unavailable_ale_position(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<Position> {
        let object = self
            .observation_ai(owner)
            .base
            .interesting_object
            .expect("ale availability requires bottle");
        let object_id = self.expect_entity_id_for_index(object.get(), "ale availability bottle");
        let position = self.observation_object_position(object_id);
        if !self
            .expect_entity(object_id, "ale availability active")
            .element_data()
            .active
        {
            return Some(position);
        }
        let me = self.live_ai_position(owner);
        let dx = me.x - position.x;
        let dy = me.y - position.y;
        let my_distance = dx * dx + dy * dy;
        let count = self.entities().len();
        for index in 0..count {
            let Some((id, entity)) = self.entities().get_legacy_slot(index as u32) else {
                continue;
            };
            if id == owner || !entity.is_npc() {
                continue;
            }
            let ai = entity
                .ai_controller()
                .expect("ale competitor requires NPC brain");
            let state = ai.current_substate;
            if !matches!(
                state,
                Substate::WonderingApproachingAle
                    | Substate::WonderingAleReactiontime
                    | Substate::WonderingDrinkingAle
            ) || ai.interesting_object != Some(object)
            {
                continue;
            }
            if !self.live_ai_detects_180(assets, owner, id) {
                continue;
            }
            let friend = self.live_ai_position(id);
            if state == Substate::WonderingDrinkingAle || {
                let dx = friend.x - position.x;
                let dy = friend.y - position.y;
                dx * dx + dy * dy < my_distance
            } {
                return Some(self.live_ai_position(id));
            }
        }
        None
    }
    #[cfg(test)]
    pub(super) fn execute_ai_ale_approach(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        arrived: bool,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_ale_approach(arrived)
    }

    #[cfg(test)]
    pub(super) fn execute_ai_ale_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        AiOwnerCtx::new(self, sim, assets, owner).execute_ai_ale_reaction()
    }

    pub(in crate::engine) fn reliable_ale_for_actor(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        if !self
            .control
            .sim_config
            .item_gameplay
            .ale_reliable_distraction
        {
            return false;
        }
        let Entity::Soldier(soldier) = self.expect_entity(owner, "ale reliability owner") else {
            return false;
        };
        !assets
            .profile_manager
            .get_soldier(soldier.soldier.soldier_profile_index)
            .unwrap_or_else(|| {
                panic!(
                    "ale reliability requires soldier profile {:?} for {owner:?}",
                    soldier.soldier.soldier_profile_index
                )
            })
            .vip
    }

    fn observation_object_position(&self, object: EntityId) -> Position {
        let entity = self.expect_entity(object, "retained observation object");
        entity
            .object_data()
            .expect("observation object handle requires an object");
        let element = entity.element_data();
        let point = element.position_map();
        Position {
            x: point.x,
            y: point.y,
            sector: element.sector(),
            level: element.layer(),
        }
    }
}

impl AiOwnerCtx<'_> {
    pub(super) fn execute_ai_seen_enemy_as_archer(&mut self, target: crate::ai::HumanHandle) {
        self.engine.reinitialize_live_ai_enemies(self.owner);
        let target = self
            .engine
            .expect_human_id_for_ai_handle(target, "archer sighting target");
        let below = self.engine.observation_enemy_below(self.owner, target);
        self.engine.observation_ai_mut(self.owner).enemy_seen_below = below;
        self.execute_battle_decisions();
    }

    pub(super) fn observation_stop(&mut self) {
        self.stop_ai_owner();
    }

    pub(super) fn observation_say(&mut self, remark: Remark) {
        self.execute_ai_speech(crate::ai::AiSpeechAttempt { remark, flags: 0 });
    }

    pub(super) fn observation_face_entity(&mut self, target: EntityId, fast: bool) {
        let position = self.engine.live_ai_position(target);
        let elevation = self
            .engine
            .expect_entity(target, "observation facing target")
            .element_data()
            .position()
            .z as i16;
        self.duty_face_position_signed_elevation(position, elevation, fast);
    }

    pub(super) fn execute_ai_react_live(&mut self, maximum: u16) {
        let entity = self.engine.expect_entity(self.owner, "reaction owner");
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
                    .observation_ai(self.owner)
                    .profile(&self.assets.profile_manager)
                    .intelligence as f32)
                * 0.01
                * f32::from(maximum)
                * modifier
                + 1.0) as u32
        };
        self.engine.observation_timer(self.owner, frames);
    }

    pub(super) fn execute_ai_seen_enemy(&mut self, target: u32) {
        if i32::from(self.engine.observation_ai(self.owner).base.blood_alcohol)
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !self
                .engine
                .observation_ai(self.owner)
                .has_the_new_task_priority()
        {
            return;
        }
        let ai = self.engine.observation_ai_mut(self.owner);
        ai.current_task_priority = ai.new_task_priority;
        ai.investigating_distraction = false;
        let enemy = self
            .engine
            .expect_human_id_for_ai_handle(target, "seen enemy");
        let me = self.engine.expect_entity(self.owner, "enemy observer");
        let other = self.engine.expect_entity(enemy, "seen enemy");
        let royalist = me.camp() == Camp::Royalists;
        if royalist
            && (other.is_unconscious()
                || other.element_data().posture() == crate::element::Posture::Tied
                || other.human_data().expect("seen human").carrier.is_some())
        {
            return;
        }
        if matches!(other, Entity::Pc(pc) if pc.pc.guard.is_some()) {
            return;
        }
        if royalist
            && other.element_data().position().z > me.element_data().position().z + 100.0
            && matches!(other, Entity::Soldier(_))
            && !other.enemy_ai().expect("seen soldier brain").is_archer()
        {
            return;
        }
        let below = self.engine.observation_enemy_below(self.owner, enemy);
        let frame = self.engine.control.frame_counter;
        let ai = self.engine.observation_ai_mut(self.owner);
        ai.base.frame_when_enemy_detected = frame;
        ai.enemy_seen_below = below;
        self.engine
            .ai_actor_mut(self.owner, "accepted enemy sighting")
            .alerted = true;

        self.engine.observation_forget_object(self.owner);
        let position = self.engine.live_ai_position(enemy);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, position);
        if self.engine.entity_data_in_building_sector(
            self.engine
                .expect_entity(self.owner, "sighting building")
                .element_data(),
        ) {
            self.engine
                .dispatch_enemy_in_house_alert(self.sim, self.owner, self.assets);
            return;
        }
        self.engine.reinitialize_live_ai_enemies(self.owner);
        let ai = self.engine.observation_ai_mut(self.owner);
        if ai.pc_missed && ai.missed_pc == Some(AiEntityHandle::new(target)) {
            ai.pc_missed = false;
        }
        let hint = self.engine.live_ai_position(enemy);
        self.engine
            .execute_ai_look_there(self.sim, self.assets, self.owner, hint, 100);
        if self
            .engine
            .expect_entity(self.owner, "post-sighting action")
            .actor_data()
            .expect("sighting actor")
            .action_state
            == crate::element::ActionState::MovingFast
        {
            self.duty_set_state(AiState::Attacking, Substate::AttackingReactiontimeRunning);
            self.engine
                .observation_ai_mut(self.owner)
                .base
                .primary_target = Some(AiEntityHandle::new(target));
            self.engine
                .observation_focus(self.owner, Some(AiEntityHandle::new(target)));
            self.engine.reinitialize_live_ai_enemies(self.owner);
            let position = self.engine.live_ai_position(enemy);
            let a = self
                .engine
                .expect_entity(self.owner, "sighting distance owner")
                .element_data()
                .position();
            let b = self
                .engine
                .expect_entity(enemy, "sighting distance target")
                .element_data()
                .position();
            let dx = b.x - a.x;
            let dy = (b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = b.z - a.z;
            let radius = ((dx * dx + dy * dy + dz * dz).sqrt() / 3.0) as u16;
            self.duty_go_near(position, i32::from(radius), GotoFlags::RUN);
            self.engine.observation_timer(self.owner, 10);
        } else {
            self.observation_stop();
            self.observation_say(Remark::SeesEnemy);
            self.engine
                .observation_ai_mut(self.owner)
                .base
                .primary_target = Some(AiEntityHandle::new(target));
            self.engine
                .observation_focus(self.owner, Some(AiEntityHandle::new(target)));
            self.engine.reinitialize_live_ai_enemies(self.owner);
            let primary = self
                .engine
                .observation_ai(self.owner)
                .base
                .primary_target
                .expect("sighting current primary");
            let primary = self
                .engine
                .expect_human_id_for_ai_handle(primary.get(), "sighting current primary");
            let a = self
                .engine
                .expect_entity(self.owner, "sighting near owner")
                .element_data()
                .position();
            let b = self
                .engine
                .expect_entity(primary, "sighting near target")
                .element_data()
                .position();
            let distance = (b.x - a.x)
                .abs()
                .max(((b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
                .max((b.z - a.z).abs());
            if distance < 50.0 {
                self.duty_set_state(AiState::Attacking, Substate::AttackingReactiontime);
                self.execute_battle_decisions();
            } else if self.engine.observation_ai(self.owner).enemy_seen_below {
                self.duty_set_state(AiState::Attacking, Substate::AttackingReactiontime);
                self.engine.observation_timer(self.owner, 5);
            } else {
                self.duty_set_state(AiState::Attacking, Substate::AttackingReactiontimeTurning);
                self.observation_face_entity(enemy, true);
                self.engine.observation_timer(self.owner, 20);
            }
        }
    }

    pub(super) fn execute_ai_seen_shadow(&mut self, position: Position) {
        let me = self.engine.expect_entity(self.owner, "shadow observer");
        if self
            .engine
            .entity_data_in_building_sector(me.element_data())
            || me.element_data().posture() == crate::element::Posture::LeaningOut
        {
            return;
        }
        self.observation_stop();
        self.duty_set_state(AiState::Default, Substate::DefaultLookingShadow);
        self.engine.execute_ai_set_alert_status(
            self.assets,
            self.owner,
            crate::ai::AlertLevel::Yellow,
            crate::ai::AlertFlags::ONLY_MUSIC,
        );

        self.duty_face_position_ground(position);
        self.engine.observation_timer(self.owner, 10);
    }

    pub(super) fn execute_ai_received_arrow(&mut self, origin: Position) {
        self.engine
            .observation_ai_mut(self.owner)
            .current_task_priority = task_priority::ENEMY;
        self.engine.observation_forget_object(self.owner);
        self.observation_stop();
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, origin);
        let ai = self.engine.observation_ai(self.owner);
        let seeking = ai.base.current_state == AiState::Seeking
            && ai.get_rank(&self.assets.profile_manager) != ProfileRank::Officer;
        let rank = ai.get_rank(&self.assets.profile_manager);
        if !seeking
            && !matches!(
                rank,
                ProfileRank::Soldier | ProfileRank::Knight | ProfileRank::Officer
            )
        {
            return;
        }
        if !seeking {
            self.engine.observation_emoticon(self.owner);
        }
        let substate = if !seeking && rank == ProfileRank::Officer {
            Substate::SeekingArrowJustWatching
        } else {
            Substate::SeekingArrowReactiontime
        };
        self.duty_set_state(AiState::Seeking, substate);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .seek_position = origin;
        let mut point = self.engine.observation_ai(self.owner).base.seek_position;
        let here = self.engine.live_ai_position(self.owner);
        self.engine
            .ai
            .global
            .set_pos_on_near_seek_point(self.sim, here, &mut point, 0.3, 0);
        self.engine
            .observation_ai_mut(self.owner)
            .base
            .seek_position = point;
        self.duty_face_position_ground(point);
        if seeking {
            self.engine.observation_timer(self.owner, 1);
            return;
        }
        let object = self
            .engine
            .observation_ai(self.owner)
            .base
            .interesting_object;
        self.engine.observation_focus(self.owner, object);
        self.engine
            .execute_ai_look_there(self.sim, self.assets, self.owner, origin, 200);
        if rank == ProfileRank::Officer {
            self.engine
                .observation_timer(self.owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
        } else {
            self.execute_ai_react_live(
                crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16 + 50,
            );
        }
    }

    pub(super) fn execute_ai_seen_object(&mut self, object: u32) {
        let target = self
            .engine
            .expect_entity_id_for_index(object, "seen object");
        let object_type = self
            .engine
            .expect_entity(target, "seen object type")
            .object_data()
            .expect("seen object entity")
            .object_type;
        let substate = self.engine.observation_ai(self.owner).base.current_substate;
        match object_type {
            crate::element::ObjectType::Coin | crate::element::ObjectType::Purse => {
                if substate.is_take_money()
                    || substate.is_fight_for_money()
                    || matches!(
                        substate,
                        Substate::WonderingSoldierLookingOfficerWhoFinishedBrawl
                            | Substate::WonderingApproachingBrawlVictim
                            | Substate::WonderingAwakenBrawlVictim
                    )
                {
                    self.engine
                        .observation_ai_mut(self.owner)
                        .other_seen_money
                        .push(object);
                    return;
                }
                self.observation_stop();
                self.observation_say(Remark::SeesObject);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .interesting_object = Some(AiEntityHandle::new(object));
                self.observation_face_object(target);
                self.engine.observation_emoticon(self.owner);
                self.duty_set_state(AiState::Wondering, Substate::WonderingMoneyReactiontime);
                let object = self
                    .engine
                    .observation_ai(self.owner)
                    .base
                    .interesting_object;
                self.engine.observation_focus(self.owner, object);
                let delay = if self
                    .engine
                    .observation_ai(self.owner)
                    .get_rank(&self.assets.profile_manager)
                    == ProfileRank::Officer
                {
                    60
                } else {
                    30
                };
                self.engine.observation_timer(self.owner, delay);
            }
            crate::element::ObjectType::Ale => {
                if substate.is_take_ale() {
                    self.engine
                        .observation_ai_mut(self.owner)
                        .other_seen_ale
                        .push(object);
                    return;
                }
                self.engine.execute_ai_break_macro(self.owner);

                self.observation_say(Remark::SeesObject);
                self.observation_face_object(target);
                self.engine.observation_emoticon(self.owner);
                self.engine
                    .observation_ai_mut(self.owner)
                    .base
                    .interesting_object = Some(AiEntityHandle::new(object));
                self.engine
                    .observation_focus(self.owner, Some(AiEntityHandle::new(object)));
                self.duty_set_state(AiState::Wondering, Substate::WonderingAleReactiontime);
                self.execute_ai_react_live(crate::parameters_ai::AI_FIRST_LOOK_TIME as u16);
            }
            _ => {}
        }
    }

    pub(super) fn execute_ai_ale_approach(&mut self, arrived: bool) {
        if let Some(position) = self
            .engine
            .unavailable_ale_position(self.assets, self.owner)
        {
            self.duty_face_position_ground(position);
            self.engine
                .observation_ai_mut(self.owner)
                .base
                .set_emoticon(EmoticonType::Thunderstorm);

            self.duty_set_state(AiState::Wondering, Substate::WonderingAleAway);
            self.engine.observation_timer(self.owner, 30);
        } else if arrived {
            let object = self
                .engine
                .observation_ai(self.owner)
                .base
                .interesting_object
                .expect("ale arrival requires bottle");
            let object = self
                .engine
                .expect_entity_id_for_index(object.get(), "ale arrival bottle");
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::DrinkAle,
                Some(self.owner),
                Some(object),
            ));
            self.engine.launch_sequence(self.sim, self.assets, sequence);

            self.duty_set_state(AiState::Wondering, Substate::WonderingDrinkingAle);
        } else {
            self.engine.observation_timer(self.owner, 20);
        }
    }

    pub(super) fn execute_ai_ale_reaction(&mut self) {
        let ai = self.engine.observation_ai(self.owner);
        let actor = self.engine.expect_entity(self.owner, "ale reaction owner");
        let take = i32::from(ai.base.blood_alcohol)
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || (actor.element_data().active
                && !self
                    .engine
                    .entity_data_in_building_sector(actor.element_data())
                && (ai.profile(&self.assets.profile_manager).beer > 0
                    || self.engine.reliable_ale_for_actor(self.assets, self.owner)));
        if take {
            let ai = self.engine.observation_ai_mut(self.owner);
            ai.base.object_of_desire = ai.base.interesting_object;
            self.duty_set_state(AiState::Wondering, Substate::WonderingApproachingAle);
            let frame = self.engine.control.frame_counter;
            self.engine
                .observation_ai_mut(self.owner)
                .base
                .set_transient_emoticon(EmoticonType::Sun, 20, frame);

            self.observation_say(Remark::AleYes);
            let target = self
                .engine
                .observation_ai(self.owner)
                .base
                .interesting_object
                .expect("ale reaction requires retained bottle");
            let target = self
                .engine
                .expect_entity_id_for_index(target.get(), "ale reaction bottle");
            let position = self.engine.observation_object_position(target);
            self.duty_go_near(
                position,
                crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::FIND_ACCESSIBLE,
            );
            let here = self.engine.live_ai_position(self.owner);
            self.engine
                .observation_ai_mut(self.owner)
                .return_to_patrol_point = here;
            self.engine.observation_timer(self.owner, 20);
        } else {
            let frame = self.engine.control.frame_counter;
            self.engine
                .observation_ai_mut(self.owner)
                .base
                .set_transient_emoticon(EmoticonType::Cloud, 50, frame);

            let remark = if self
                .engine
                .expect_entity(self.owner, "ale refusal owner")
                .is_vip()
            {
                Remark::VipAleNo
            } else {
                Remark::AleNo
            };
            self.observation_say(remark);
            self.execute_ai_return_to_duty(crate::ai::DutyFlags::KEEP_EMOTICON);
        }
    }

    fn observation_face_object(&mut self, object: EntityId) {
        let position = self.engine.observation_object_position(object);
        let elevation = self
            .engine
            .expect_entity(object, "object facing elevation")
            .element_data()
            .position()
            .z as u16 as i16;
        self.duty_face_position_signed_elevation(position, elevation, false);
    }
}
