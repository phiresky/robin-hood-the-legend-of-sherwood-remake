//! Sight and arrow reactions retain actor identities across synchronous callbacks.
use super::*;
use crate::ai::{
    AiEntityHandle, AiState, EmoticonType, GotoFlags, Position, Remark, ReportType, Substate,
};
use crate::ai_enemy::{EnemyAi, ProfileRank, task_priority};
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
    pub(super) fn execute_ai_seen_enemy_as_archer(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: crate::ai::HumanHandle,
    ) {
        self.reinitialize_live_ai_enemies(owner);
        let target = self.expect_human_id_for_ai_handle(target, "archer sighting target");
        let below = self.observation_enemy_below(owner, target);
        self.observation_ai_mut(owner).enemy_seen_below = below;
        self.execute_battle_decisions(sim, assets, owner);
    }
    pub(super) fn observation_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.observation_ai_mut(owner)
            .base
            .launch_timer(frames, frame);
    }
    pub(super) fn observation_stop(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.stop_ai_owner(sim, assets, owner);
    }
    pub(super) fn observation_say(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        remark: Remark,
    ) {
        self.execute_ai_speech(
            sim,
            assets,
            owner,
            crate::ai::AiSpeechAttempt { remark, flags: 0 },
        );
    }
    pub(super) fn observation_face_entity(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
        fast: bool,
    ) {
        let position = self.live_ai_position(target);
        let elevation = self
            .expect_entity(target, "observation facing target")
            .element_data()
            .position()
            .z as i16;
        self.duty_face_position_signed_elevation(sim, assets, owner, position, elevation, fast);
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
    pub(super) fn execute_ai_react_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        maximum: u16,
    ) {
        let entity = self.expect_entity(owner, "reaction owner");
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
                    .observation_ai(owner)
                    .profile(&assets.profile_manager)
                    .intelligence as f32)
                * 0.01
                * f32::from(maximum)
                * modifier
                + 1.0) as u32
        };
        self.observation_timer(owner, frames);
    }

    pub(super) fn execute_ai_seen_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: u32,
    ) {
        if i32::from(self.observation_ai(owner).base.blood_alcohol)
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || !self.observation_ai(owner).has_the_new_task_priority()
        {
            return;
        }
        let ai = self.observation_ai_mut(owner);
        ai.current_task_priority = ai.new_task_priority;
        ai.investigating_distraction = false;
        let enemy = self.expect_human_id_for_ai_handle(target, "seen enemy");
        let me = self.expect_entity(owner, "enemy observer");
        let other = self.expect_entity(enemy, "seen enemy");
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
        let below = self.observation_enemy_below(owner, enemy);
        let frame = self.control.frame_counter;
        let ai = self.observation_ai_mut(owner);
        ai.base.frame_when_enemy_detected = frame;
        ai.enemy_seen_below = below;
        self.ai_actor_mut(owner, "accepted enemy sighting").alerted = true;

        self.observation_forget_object(owner);
        let position = self.live_ai_position(enemy);
        self.observation_ai_mut(owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, position);
        if self.entity_data_in_building_sector(
            self.expect_entity(owner, "sighting building")
                .element_data(),
        ) {
            self.dispatch_enemy_in_house_alert(sim, owner, assets);
            return;
        }
        self.reinitialize_live_ai_enemies(owner);
        let ai = self.observation_ai_mut(owner);
        if ai.pc_missed && ai.missed_pc == Some(AiEntityHandle::new(target)) {
            ai.pc_missed = false;
        }
        let hint = self.live_ai_position(enemy);
        self.execute_ai_look_there(sim, assets, owner, hint, 100);
        if self
            .expect_entity(owner, "post-sighting action")
            .actor_data()
            .expect("sighting actor")
            .action_state
            == crate::element::ActionState::MovingFast
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingReactiontimeRunning,
            );
            self.observation_ai_mut(owner).base.primary_target = Some(AiEntityHandle::new(target));
            self.observation_focus(owner, Some(AiEntityHandle::new(target)));
            self.reinitialize_live_ai_enemies(owner);
            let position = self.live_ai_position(enemy);
            let a = self
                .expect_entity(owner, "sighting distance owner")
                .element_data()
                .position();
            let b = self
                .expect_entity(enemy, "sighting distance target")
                .element_data()
                .position();
            let dx = b.x - a.x;
            let dy = (b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = b.z - a.z;
            let radius = ((dx * dx + dy * dy + dz * dz).sqrt() / 3.0) as u16;
            self.duty_go_near(
                sim,
                assets,
                owner,
                position,
                i32::from(radius),
                GotoFlags::RUN,
            );
            self.observation_timer(owner, 10);
        } else {
            self.observation_stop(sim, assets, owner);
            self.observation_say(sim, assets, owner, Remark::SeesEnemy);
            self.observation_ai_mut(owner).base.primary_target = Some(AiEntityHandle::new(target));
            self.observation_focus(owner, Some(AiEntityHandle::new(target)));
            self.reinitialize_live_ai_enemies(owner);
            let primary = self
                .observation_ai(owner)
                .base
                .primary_target
                .expect("sighting current primary");
            let primary =
                self.expect_human_id_for_ai_handle(primary.get(), "sighting current primary");
            let a = self
                .expect_entity(owner, "sighting near owner")
                .element_data()
                .position();
            let b = self
                .expect_entity(primary, "sighting near target")
                .element_data()
                .position();
            let distance = (b.x - a.x)
                .abs()
                .max(((b.y - a.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs())
                .max((b.z - a.z).abs());
            if distance < 50.0 {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingReactiontime,
                );
                self.execute_battle_decisions(sim, assets, owner);
            } else if self.observation_ai(owner).enemy_seen_below {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingReactiontime,
                );
                self.observation_timer(owner, 5);
            } else {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingReactiontimeTurning,
                );
                self.observation_face_entity(sim, assets, owner, enemy, true);
                self.observation_timer(owner, 20);
            }
        }
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
    pub(super) fn execute_ai_seen_shadow(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        position: Position,
    ) {
        let me = self.expect_entity(owner, "shadow observer");
        if self.entity_data_in_building_sector(me.element_data())
            || me.element_data().posture() == crate::element::Posture::LeaningOut
        {
            return;
        }
        self.observation_stop(sim, assets, owner);
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Default,
            Substate::DefaultLookingShadow,
        );
        self.execute_ai_set_alert_status(
            assets,
            owner,
            crate::ai::AlertLevel::Yellow,
            crate::ai::AlertFlags::ONLY_MUSIC,
        );

        self.duty_face_position_ground(sim, assets, owner, position);
        self.observation_timer(owner, 10);
    }
    pub(super) fn execute_ai_received_arrow(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        origin: Position,
    ) {
        self.observation_ai_mut(owner).current_task_priority = task_priority::ENEMY;
        self.observation_forget_object(owner);
        self.observation_stop(sim, assets, owner);
        self.observation_ai_mut(owner)
            .base
            .my_reconnaissance_report
            .update(ReportType::Enemy, origin);
        let ai = self.observation_ai(owner);
        let seeking = ai.base.current_state == AiState::Seeking
            && ai.get_rank(&assets.profile_manager) != ProfileRank::Officer;
        let rank = ai.get_rank(&assets.profile_manager);
        if !seeking
            && !matches!(
                rank,
                ProfileRank::Soldier | ProfileRank::Knight | ProfileRank::Officer
            )
        {
            return;
        }
        if !seeking {
            self.observation_emoticon(owner);
        }
        let substate = if !seeking && rank == ProfileRank::Officer {
            Substate::SeekingArrowJustWatching
        } else {
            Substate::SeekingArrowReactiontime
        };
        self.duty_set_state(sim, assets, owner, AiState::Seeking, substate);
        self.observation_ai_mut(owner).base.seek_position = origin;
        let mut point = self.observation_ai(owner).base.seek_position;
        let here = self.live_ai_position(owner);
        self.ai
            .global
            .set_pos_on_near_seek_point(sim, here, &mut point, 0.3, 0);
        self.observation_ai_mut(owner).base.seek_position = point;
        self.duty_face_position_ground(sim, assets, owner, point);
        if seeking {
            self.observation_timer(owner, 1);
            return;
        }
        let object = self.observation_ai(owner).base.interesting_object;
        self.observation_focus(owner, object);
        self.execute_ai_look_there(sim, assets, owner, origin, 200);
        if rank == ProfileRank::Officer {
            self.observation_timer(owner, crate::parameters_ai::AI_FIRST_LOOK_TIME as u32);
        } else {
            self.execute_ai_react_live(
                sim,
                assets,
                owner,
                crate::parameters_ai::AI_MAX_STANDARD_REACTIONTIME as u16 + 50,
            );
        }
    }
    pub(super) fn execute_ai_seen_object(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        object: u32,
    ) {
        let target = self.expect_entity_id_for_index(object, "seen object");
        let object_type = self
            .expect_entity(target, "seen object type")
            .object_data()
            .expect("seen object entity")
            .object_type;
        let substate = self.observation_ai(owner).base.current_substate;
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
                    self.observation_ai_mut(owner).other_seen_money.push(object);
                    return;
                }
                self.observation_stop(sim, assets, owner);
                self.observation_say(sim, assets, owner, Remark::SeesObject);
                self.observation_ai_mut(owner).base.interesting_object =
                    Some(AiEntityHandle::new(object));
                self.observation_face_object(sim, assets, owner, target);
                self.observation_emoticon(owner);
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingMoneyReactiontime,
                );
                let object = self.observation_ai(owner).base.interesting_object;
                self.observation_focus(owner, object);
                let delay = if self.observation_ai(owner).get_rank(&assets.profile_manager)
                    == ProfileRank::Officer
                {
                    60
                } else {
                    30
                };
                self.observation_timer(owner, delay);
            }
            crate::element::ObjectType::Ale => {
                if substate.is_take_ale() {
                    self.observation_ai_mut(owner).other_seen_ale.push(object);
                    return;
                }
                self.execute_ai_break_macro(owner);

                self.observation_say(sim, assets, owner, Remark::SeesObject);
                self.observation_face_object(sim, assets, owner, target);
                self.observation_emoticon(owner);
                self.observation_ai_mut(owner).base.interesting_object =
                    Some(AiEntityHandle::new(object));
                self.observation_focus(owner, Some(AiEntityHandle::new(object)));
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Wondering,
                    Substate::WonderingAleReactiontime,
                );
                self.execute_ai_react_live(
                    sim,
                    assets,
                    owner,
                    crate::parameters_ai::AI_FIRST_LOOK_TIME as u16,
                );
            }
            _ => {}
        }
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
    pub(super) fn execute_ai_ale_approach(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        arrived: bool,
    ) {
        if let Some(position) = self.unavailable_ale_position(assets, owner) {
            self.duty_face_position_ground(sim, assets, owner, position);
            self.observation_ai_mut(owner)
                .base
                .set_emoticon(EmoticonType::Thunderstorm);

            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingAleAway,
            );
            self.observation_timer(owner, 30);
        } else if arrived {
            let object = self
                .observation_ai(owner)
                .base
                .interesting_object
                .expect("ale arrival requires bottle");
            let object = self.expect_entity_id_for_index(object.get(), "ale arrival bottle");
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::DrinkAle,
                Some(owner),
                Some(object),
            ));
            self.launch_sequence(sim, assets, sequence);

            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingDrinkingAle,
            );
        } else {
            self.observation_timer(owner, 20);
        }
    }

    pub(super) fn execute_ai_ale_reaction(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let ai = self.observation_ai(owner);
        let actor = self.expect_entity(owner, "ale reaction owner");
        let take = i32::from(ai.base.blood_alcohol)
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || (actor.element_data().active
                && !self.entity_data_in_building_sector(actor.element_data())
                && (ai.profile(&assets.profile_manager).beer > 0
                    || self.reliable_ale_for_actor(assets, owner)));
        if take {
            let ai = self.observation_ai_mut(owner);
            ai.base.object_of_desire = ai.base.interesting_object;
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingApproachingAle,
            );
            let frame = self.control.frame_counter;
            self.observation_ai_mut(owner).base.set_transient_emoticon(
                EmoticonType::Sun,
                20,
                frame,
            );

            self.observation_say(sim, assets, owner, Remark::AleYes);
            let target = self
                .observation_ai(owner)
                .base
                .interesting_object
                .expect("ale reaction requires retained bottle");
            let target = self.expect_entity_id_for_index(target.get(), "ale reaction bottle");
            let position = self.observation_object_position(target);
            self.duty_go_near(
                sim,
                assets,
                owner,
                position,
                crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::FIND_ACCESSIBLE,
            );
            let here = self.live_ai_position(owner);
            self.observation_ai_mut(owner).return_to_patrol_point = here;
            self.observation_timer(owner, 20);
        } else {
            let frame = self.control.frame_counter;
            self.observation_ai_mut(owner).base.set_transient_emoticon(
                EmoticonType::Cloud,
                50,
                frame,
            );

            let remark = if self.expect_entity(owner, "ale refusal owner").is_vip() {
                Remark::VipAleNo
            } else {
                Remark::AleNo
            };
            self.observation_say(sim, assets, owner, remark);
            self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::KEEP_EMOTICON);
        }
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

    fn observation_face_object(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        object: EntityId,
    ) {
        let position = self.observation_object_position(object);
        let elevation = self
            .expect_entity(object, "object facing elevation")
            .element_data()
            .position()
            .z as u16 as i16;
        self.duty_face_position_signed_elevation(sim, assets, owner, position, elevation, false);
    }
}
