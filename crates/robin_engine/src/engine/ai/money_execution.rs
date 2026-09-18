//! Money-fight scans and their synchronous caller tails.

use super::*;
use crate::ai::{AiEntityHandle, AiState, DutyFlags, GotoFlags, MoneyFightOperation, Substate};
use crate::ai_enemy::EnemyAi;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(xs: &[f32]) -> (EngineInner, LevelAssets, Vec<EntityId>) {
        let mut engine = EngineInner::new();
        let (sector, _) = crate::engine::test_support::extra_engine_combat::square_sector_map(
            &mut engine,
            (128, 128),
            (2000.0, 2000.0),
        );
        let ids: Vec<_> = xs
            .iter()
            .map(|&x| {
                let mut entity =
                    crate::engine::test_support::actors::make_test_ai_soldier(Camp::Lacklandists);
                entity
                    .element_data_mut()
                    .set_position_map(MapPoint::new(x, 500.0));
                entity
                    .element_data_mut()
                    .set_position(crate::coordinates::WorldPoint3D::new(x, 500.0, 0.0));
                entity.element_data_mut().set_sector(Some(sector));
                entity.ai_actor_data_mut().unwrap().view_radius = 500;
                entity.npc_data_mut().unwrap().life_points = 100;
                let id = engine.add_test_entity(entity);
                let ai = engine.money_ai_mut(id);
                ai.base.owner_entity_id = Some(id);
                ai.base.me = id.index();
                id
            })
            .collect();
        engine
            .world
            .soldier_registry
            .rebuild_from_order(&engine.world.entities, ids.iter().copied());
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, assets, ids)
    }

    fn knockout(engine: &mut EngineInner, target: EntityId) {
        engine
            .world
            .entities
            .get_mut(target)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .unconscious = true;
        let ai = engine.money_ai_mut(target);
        ai.base.current_substate = Substate::SleepingUnconscious;
        ai.base.knocked_out_in_money_fight = true;
    }

    #[test]
    fn victim_scan_includes_owner_and_inserts_later_equal_distance_first() {
        let (mut engine, assets, ids) = fixture(&[500.0, 400.0, 600.0]);
        for &target in &ids {
            knockout(&mut engine, target);
        }
        engine.create_live_money_fight_victims(&assets, ids[0]);
        assert_eq!(
            engine.money_ai(ids[0]).money_fight_victims,
            vec![ids[0].index(), ids[2].index(), ids[1].index()]
        );
    }

    #[test]
    fn money_morale_reads_current_knockout_flags_and_skips_dead_candidates() {
        let (mut engine, mut assets, ids) = fixture(&[500.0, 600.0, 700.0, 800.0, 900.0]);
        crate::engine::test_support::actors::edit_enemy_profile(
            &mut assets,
            engine.money_ai_mut(ids[0]),
            |profile| profile.money = 40,
        );
        engine.money_ai_mut(ids[1]).base.current_substate = Substate::WonderingBrawlHitting;
        knockout(&mut engine, ids[2]);
        knockout(&mut engine, ids[3]);
        knockout(&mut engine, ids[4]);
        engine
            .world
            .entities
            .get_mut(ids[4])
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .life_points = 0;
        assert!(!engine.wants_live_money_fight(&assets, ids[0]));
        engine.money_ai_mut(ids[3]).base.knocked_out_in_money_fight = false;
        assert!(engine.wants_live_money_fight(&assets, ids[0]));
    }

    #[test]
    fn patrol_scan_overwrites_a_money_sort_key_even_when_member_is_rejected() {
        let (mut engine, assets, ids) = fixture(&[500.0, 400.0, 800.0]);
        knockout(&mut engine, ids[1]);
        engine.create_live_money_fight_victims(&assets, ids[0]);
        assert_eq!(
            engine
                .expect_entity(ids[1], "money key")
                .human_data()
                .unwrap()
                .sorting_distance,
            10_000.0
        );
        engine.money_ai_mut(ids[2]).base.theoretical_patrol = vec![ids[1]];
        engine.initialize_patrol_for_npc(&assets, ids[2]);
        assert_eq!(
            engine
                .expect_entity(ids[1], "nested patrol key")
                .human_data()
                .unwrap()
                .sorting_distance,
            160_000.0
        );
        assert!(engine.money_ai(ids[2]).base.patrol.is_empty());
        assert_eq!(
            engine.money_ai(ids[0]).money_fight_victims,
            [ids[1].index()]
        );
    }

    #[test]
    fn sorting_keys_survive_native_rollback_but_not_persisted_saves_or_hashes() {
        use robin_util::state_hash::StateHash;
        use std::hash::Hasher;
        let mut human = crate::element::HumanData::default();
        let json = serde_json::to_string(&human).unwrap();
        let binary = bitcode::encode(&human);
        let mut before = std::collections::hash_map::DefaultHasher::new();
        human.state_hash(&mut before);
        human.sorting_distance = 160_000.0;
        let mut after = std::collections::hash_map::DefaultHasher::new();
        human.state_hash(&mut after);
        assert_eq!(before.finish(), after.finish());
        assert_eq!(human.clone().sorting_distance, 160_000.0);
        assert_eq!(serde_json::to_string(&human).unwrap(), json);
        assert_ne!(bitcode::encode(&human), binary);
        assert_eq!(
            bitcode::decode::<crate::element::HumanData>(&bitcode::encode(&human))
                .unwrap()
                .sorting_distance,
            160_000.0
        );
        assert_eq!(
            serde_json::from_str::<crate::element::HumanData>(&json)
                .unwrap()
                .sorting_distance,
            0.0
        );
        assert_eq!(
            bitcode::decode::<crate::element::HumanData>(&binary)
                .unwrap()
                .sorting_distance,
            0.0
        );
    }

    #[test]
    fn enemy_rebuild_skips_unconscious_candidates_before_visibility() {
        let (mut engine, assets, ids) = fixture(&[500.0, 600.0]);
        knockout(&mut engine, ids[1]);
        engine.money_ai_mut(ids[1]).base.current_substate = Substate::WonderingBrawlHitting;
        crate::sight_obstacle::begin_parity_visibility_capture();
        engine.create_live_money_fight_enemies(&assets, ids[0]);
        assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
        assert!(engine.money_ai(ids[0]).money_fight_enemies.is_empty());
    }
}

impl EngineInner {
    fn money_ai(&self, owner: EntityId) -> &EnemyAi {
        self.enemy_ai(owner, "money-fight owner")
    }

    fn money_ai_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.enemy_ai_mut(owner, "money-fight owner")
    }

    fn money_camp_soldier(&self, camp: Camp, index: usize) -> EntityId {
        EntityId::Soldier(crate::entity_id::SoldierId(
            self.world.soldier_registry.camp(camp)[index],
        ))
    }

    pub(in crate::engine) fn can_call_ai_soldier(&self, owner: EntityId, target: EntityId) -> bool {
        let ai = self.money_ai(target);
        if let Some(chief) = ai.base.patrol_chief
            && chief != owner
        {
            let chief_position = self.live_ai_position(chief);
            let target_position = self.live_ai_position(target);
            if (target_position.x - chief_position.x)
                .abs()
                .max((target_position.y - chief_position.y).abs())
                < 700.0
            {
                return false;
            }
        }
        ai.base
            .antagonist
            .is_none_or(|antagonist| antagonist.get() == owner.index())
    }

    fn create_live_money_fight_victims(&mut self, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "money victim scan camp").camp();
        self.money_ai_mut(owner).money_fight_victims.clear();
        // Distances are captured before each visibility query. Equal distances
        // insert before earlier candidates, including the owner when eligible.
        for index in 0..self.world.soldier_registry.camp(camp).len() {
            let target = self.money_camp_soldier(camp, index);
            let entity = self.expect_entity(target, "money-fight victim");
            if !entity.is_unconscious()
                || entity.is_dead()
                || !self.money_ai(target).base.knocked_out_in_money_fight
            {
                continue;
            }
            let here = self
                .expect_entity(owner, "money-fight victim distance owner")
                .element_data()
                .position();
            let there = entity.element_data().position();
            let dx = there.x - here.x;
            let dy = (there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = there.z - here.z;
            let square_distance = dx * dx + dy * dy + dz * dz;
            self.entities_mut()
                .expect_entity_mut(target, format_args!("money victim sorting key"))
                .human_data_mut()
                .expect("soldier human data")
                .sorting_distance = square_distance;
            if !self.patrol_member_visible(assets, owner, target) {
                continue;
            }
            let insertion = self
                .money_ai(owner)
                .money_fight_victims
                .iter()
                .position(|existing| {
                    let id = EntityId::Soldier(crate::entity_id::SoldierId(*existing));
                    !(square_distance
                        > self
                            .expect_entity(id, "prior money victim sorting key")
                            .human_data()
                            .expect("soldier human data")
                            .sorting_distance)
                })
                .unwrap_or(self.money_ai(owner).money_fight_victims.len());
            self.money_ai_mut(owner)
                .money_fight_victims
                .insert(insertion, target.index());
        }
    }

    fn wants_live_money_fight(&self, assets: &LevelAssets, owner: EntityId) -> bool {
        let camp = self.expect_entity(owner, "money morale scan camp").camp();
        if self.money_ai(owner).profile(&assets.profile_manager).money == 100
            || self.money_ai(owner).base.blood_alcohol > 0
        {
            return true;
        }
        let mut upright = 1_u16;
        let mut sleeping = 0_u16;
        for index in 0..self.world.soldier_registry.camp(camp).len() {
            let target = self.money_camp_soldier(camp, index);
            if target == owner
                || self
                    .expect_entity(target, "money-fight morale candidate")
                    .is_dead()
                || !self.patrol_member_visible(assets, owner, target)
            {
                continue;
            }
            let ai = self.money_ai(target);
            if ai.base.current_substate.is_take_money()
                || ai.base.current_substate.is_fight_for_money()
            {
                upright = upright.wrapping_add(1);
            } else if ai.base.current_substate == Substate::SleepingUnconscious
                && ai.base.knocked_out_in_money_fight
            {
                sleeping = sleeping.wrapping_add(1);
            }
        }
        (100 * u32::from(sleeping)) / (u32::from(sleeping) + u32::from(upright))
            < u32::from(self.money_ai(owner).profile(&assets.profile_manager).money)
    }

    fn create_live_money_fight_enemies(&mut self, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "money enemy scan camp").camp();
        self.money_ai_mut(owner).money_fight_enemies.clear();
        for index in 0..self.world.soldier_registry.camp(camp).len() {
            let target = self.money_camp_soldier(camp, index);
            let entity = self.expect_entity(target, "money-fight enemy candidate");
            if target == owner
                || entity.is_unconscious()
                || entity.is_dead()
                || !self.patrol_member_visible(assets, owner, target)
            {
                continue;
            }
            let substate = self.money_ai(target).base.current_substate;
            if substate.is_take_money() || substate.is_fight_for_money() {
                self.money_ai_mut(owner)
                    .money_fight_enemies
                    .push(target.index());
            }
        }
    }

    fn nearest_money_fight_enemy(&self, owner: EntityId) -> Option<u32> {
        let candidates = &self.money_ai(owner).money_fight_enemies;
        let mut nearest = *candidates.first()?;
        let here = self.live_ai_position(owner);
        let mut minimum = 65_432_u16;
        for &handle in candidates {
            let target = self.expect_human_id_for_ai_handle(handle, "nearest money-fight enemy");
            assert_ne!(target, owner, "money-fight enemy cannot be the owner");
            let position = self.live_ai_position(target);
            let mut distance = (position.x - here.x).abs().max((position.y - here.y).abs()) as u16;
            if self
                .expect_entity(target, "money-fight enemy layer")
                .element_data()
                .layer()
                != here.level
            {
                distance = distance.wrapping_add(300);
            }
            if distance < minimum {
                minimum = distance;
                nearest = handle;
            }
        }
        Some(nearest)
    }

    fn money_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.money_ai_mut(owner).base.launch_timer(frames, frame);
    }

    pub(in crate::engine) fn clean_live_seen_money(&mut self, owner: EntityId) {
        for index in (0..self.money_ai(owner).other_seen_money.len()).rev() {
            let handle = self.money_ai(owner).other_seen_money[index];
            let target = self.expect_entity_id_for_index(handle, "remembered coin");
            if !self.expect_entity(target, "remembered coin").is_active() {
                self.money_ai_mut(owner).other_seen_money.remove(index);
            }
        }
        if let Some(coin) = self.money_ai(owner).base.interesting_object {
            let target = self.expect_entity_id_for_index(coin.get(), "interesting coin");
            if !self.expect_entity(target, "interesting coin").is_active() {
                self.money_ai_mut(owner).base.interesting_object = None;
            }
        }
    }

    pub(in crate::engine) fn take_nearest_live_money(&mut self, owner: EntityId) -> Option<u32> {
        self.clean_live_seen_money(owner);
        if self.money_ai(owner).other_seen_money.is_empty() {
            return None;
        }
        let here = self.live_ai_position(owner);
        let mut minimum = 65_432_u16;
        let mut nearest = 0;
        for (index, &handle) in self.money_ai(owner).other_seen_money.iter().enumerate() {
            let target = self.expect_entity_id_for_index(handle, "nearest remembered coin");
            let position = self.live_ai_position(target);
            let mut distance = (position.x - here.x).abs().max((position.y - here.y).abs()) as u16;
            if self
                .expect_entity(target, "remembered coin layer")
                .element_data()
                .layer()
                != here.level
            {
                distance = distance.wrapping_add(300);
            }
            if distance < minimum {
                minimum = distance;
                nearest = index;
            }
        }
        Some(self.money_ai_mut(owner).other_seen_money.remove(nearest))
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_money_fight(&mut self, operation: MoneyFightOperation) {
        match operation {
            MoneyFightOperation::CleanUpAfterBrawl => {
                self.engine
                    .create_live_money_fight_victims(self.assets, self.owner);
                self.approach_next_money_victim(false);
            }
            MoneyFightOperation::CollectOrLootAfterLook => {
                self.engine
                    .create_live_money_fight_victims(self.assets, self.owner);
                while let Some(&handle) =
                    self.engine.money_ai(self.owner).money_fight_victims.first()
                {
                    let target = self
                        .engine
                        .expect_human_id_for_ai_handle(handle, "money-fight looted victim");
                    if !self.engine.money_ai(target).base.looted_after_money_fight {
                        break;
                    }
                    self.engine
                        .money_ai_mut(self.owner)
                        .money_fight_victims
                        .remove(0);
                }
                self.approach_next_money_victim(true);
            }
            MoneyFightOperation::AwakeNextVictim => self.approach_next_money_victim(false),
            MoneyFightOperation::FinishHitAfterOfficer => self.finish_money_hit(),
            MoneyFightOperation::FinishBrawl => self.finish_live_brawl(),
            MoneyFightOperation::RecoverBrawl => {
                if let Some(target) = self.engine.nearest_money_fight_enemy(self.owner) {
                    self.engine.money_ai_mut(self.owner).base.friend_in_trouble =
                        Some(AiEntityHandle::new(target));
                    self.duty_set_state(AiState::Wondering, Substate::WonderingBrawlApproaching);
                    let target = self
                        .engine
                        .money_ai(self.owner)
                        .base
                        .friend_in_trouble
                        .expect("brawl approach lost its target");
                    let target = self
                        .engine
                        .expect_human_id_for_ai_handle(target.get(), "brawl approach target");
                    self.duty_go_near(
                        self.engine.live_ai_position(target),
                        crate::parameters_ai::AI_HIT_DISTANCE,
                        GotoFlags::RUN,
                    );
                    self.execute_maybe_officer_sees_me_fighting();
                } else {
                    self.stop_live_brawl_and_collect_money();
                }
            }
            MoneyFightOperation::StolenMoney { object, thief } => {
                self.handle_stolen_money(object, thief)
            }
        }
    }

    fn approach_next_money_victim(&mut self, loot: bool) {
        if self
            .engine
            .money_ai(self.owner)
            .money_fight_victims
            .is_empty()
        {
            self.execute_ai_return_to_duty(DutyFlags::empty());
            return;
        }
        let next = self
            .engine
            .money_ai_mut(self.owner)
            .money_fight_victims
            .remove(0);
        self.engine.money_ai_mut(self.owner).base.detected_body = Some(AiEntityHandle::new(next));
        if loot {
            let victim = self
                .engine
                .expect_human_id_for_ai_handle(next, "money-fight victim to loot");
            self.engine
                .money_ai_mut(victim)
                .base
                .looted_after_money_fight = true;
        }
        let substate = if loot {
            Substate::WonderingApproachingToLoot
        } else {
            Substate::WonderingApproachingBrawlVictim
        };
        self.duty_set_state(AiState::Wondering, substate);
        let body = self
            .engine
            .money_ai(self.owner)
            .base
            .detected_body
            .expect("money victim state change lost its body");
        let body = self
            .engine
            .expect_human_id_for_ai_handle(body.get(), "money-fight victim movement");
        self.duty_go_near(
            self.engine.live_ai_position(body),
            crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
            GotoFlags::empty(),
        );
    }

    fn finish_money_hit(&mut self) {
        if let Some(friend) = self.engine.money_ai(self.owner).base.friend_in_trouble {
            let target = self
                .engine
                .expect_human_id_for_ai_handle(friend.get(), "brawl-hit partner");
            if self
                .engine
                .expect_entity(target, "brawl-hit partner")
                .is_unconscious()
            {
                self.engine
                    .money_ai_mut(self.owner)
                    .money_fight_enemies
                    .retain(|&handle| handle != friend.get());
            }
        }
        if self
            .engine
            .money_ai(self.owner)
            .money_fight_enemies
            .is_empty()
        {
            self.engine
                .create_live_money_fight_enemies(self.assets, self.owner);
        }
        if !self.engine.wants_live_money_fight(self.assets, self.owner) {
            self.engine
                .money_ai_mut(self.owner)
                .money_fight_enemies
                .clear();
            self.stop_live_brawl_and_collect_money();
        } else if self
            .engine
            .money_ai(self.owner)
            .base
            .friend_in_trouble
            .is_some_and(|friend| {
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(friend.get(), "brawl-hit surviving partner");
                !self
                    .engine
                    .expect_entity(target, "brawl-hit surviving partner")
                    .is_unconscious()
            })
        {
            self.duty_set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
            let target = self
                .engine
                .money_ai(self.owner)
                .base
                .friend_in_trouble
                .expect("brawl reaction lost partner");
            self.face_money_human(target);
            self.engine.money_timer(self.owner, 30);
        } else if let Some(target) = self.engine.nearest_money_fight_enemy(self.owner) {
            self.engine.money_ai_mut(self.owner).base.friend_in_trouble =
                Some(AiEntityHandle::new(target));
            self.duty_set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
            self.engine.money_timer(self.owner, 10);
        } else {
            self.stop_live_brawl_and_collect_money();
        }
    }

    fn finish_live_brawl(&mut self) {
        self.duty_set_state(AiState::Wondering, Substate::WonderingOfficerFinishingBrawl);
        let camp = self
            .engine
            .expect_entity(self.owner, "finish brawl scan camp")
            .camp();
        assert_eq!(
            self.engine
                .money_ai(self.owner)
                .get_rank(&self.assets.profile_manager),
            crate::profiles::ProfileRank::Officer
        );
        self.engine.money_ai_mut(self.owner).base.antagonist = None;
        self.engine.money_ai_mut(self.owner).base.list_us.clear();
        for index in 0..self.engine.world.soldier_registry.camp(camp).len() {
            let target = self.engine.money_camp_soldier(camp, index);
            let ai = self.engine.money_ai(target);
            if ai.get_rank(&self.assets.profile_manager) != crate::profiles::ProfileRank::Soldier
                || !(ai.base.current_substate.is_take_money()
                    || ai.base.current_substate.is_fight_for_money())
                || !self
                    .engine
                    .patrol_member_visible(self.assets, target, self.owner)
            {
                continue;
            }
            self.engine.execute_ai_callback(
                self.sim,
                self.assets,
                target,
                &crate::ai::Stimulus::with_human(StimulusType::CallFinishBrawl, self.owner.index()),
            );
            let ai = self.engine.money_ai_mut(self.owner);
            ai.base.list_us.push(target.index());
            if ai.base.antagonist.is_none() {
                ai.base.antagonist = Some(AiEntityHandle::new(target.index()));
            }
        }
        if let Some(target) = self.engine.money_ai(self.owner).base.antagonist {
            self.face_money_human(target);
            self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                remark: crate::ai::Remark::OfficerEndsBrawl,
                flags: crate::ai::SpeechFlags::MYTALK_1.bits(),
            });
        }
        self.engine
            .money_ai_mut(self.owner)
            .base
            .set_emoticon(crate::ai::EmoticonType::Thunderstorm);

        self.engine.money_timer(self.owner, 200);
    }

    fn face_money_human(&mut self, target: AiEntityHandle) {
        let target = self
            .engine
            .expect_human_id_for_ai_handle(target.get(), "money-fight facing target");
        let elevation = self
            .engine
            .expect_entity(target, "money-fight facing target")
            .position_iface()
            .get_elevation();
        self.duty_face_position_at_elevation(self.engine.live_ai_position(target), elevation);
    }

    fn stop_live_brawl_and_collect_money(&mut self) {
        let coin = self.engine.take_nearest_live_money(self.owner);
        self.engine.money_ai_mut(self.owner).base.interesting_object =
            coin.map(AiEntityHandle::new);
        if coin.is_some() {
            self.duty_set_state(AiState::Wondering, Substate::WonderingRunningForMoney);
            let target = self
                .engine
                .money_ai(self.owner)
                .base
                .interesting_object
                .expect("money movement lost coin");
            let target = self
                .engine
                .expect_entity_id_for_index(target.get(), "money movement coin");
            self.duty_go_near(
                self.engine.live_ai_position(target),
                crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::RUN | GotoFlags::FIND_ACCESSIBLE,
            );
        } else {
            self.duty_set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.execute_ai_look_sidewards(crate::ai::LookDirection::LeftRight);
        }
    }

    fn handle_stolen_money(&mut self, object: AiEntityHandle, thief: AiEntityHandle) {
        let object_id = self
            .engine
            .expect_entity_id_for_index(object.get(), "stolen object");
        let object_type = self
            .engine
            .expect_entity(object_id, "stolen object")
            .object_data()
            .expect("stolen item is not an object")
            .object_type;
        if !matches!(
            object_type,
            crate::element::ObjectType::Coin | crate::element::ObjectType::Purse
        ) {
            self.execute_ai_return_to_duty(DutyFlags::empty());
            return;
        }
        let target = self
            .engine
            .expect_human_id_for_ai_handle(thief.get(), "coin thief");
        if !self
            .engine
            .live_ai_detects_180(self.assets, self.owner, target)
        {
            return;
        }
        assert_ne!(self.owner, target, "coin thief cannot be the owner");
        let ai = self.engine.money_ai(self.owner);
        if ai.base.interesting_object != Some(object)
            && !ai.other_seen_money.contains(&object.get())
        {
            return;
        }
        let entity = self.engine.expect_entity(self.owner, "stolen money owner");
        let wants_money = ai.base.blood_alcohol as i32
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || (entity.is_active()
                && self
                    .engine
                    .entity_building_sector(entity.element_data().sector())
                    .is_none()
                && ai.profile(&self.assets.profile_manager).money > 0);
        if !wants_money {
            return;
        }
        if !self.engine.wants_live_money_fight(self.assets, self.owner) {
            self.engine
                .money_ai_mut(self.owner)
                .money_fight_enemies
                .clear();
            self.stop_live_brawl_and_collect_money();
            return;
        }
        let substate = self.engine.money_ai(self.owner).base.current_substate;
        if substate.is_take_money() {
            self.engine.execute_ai_break_macro(self.owner);

            self.face_money_human(thief);
            self.engine
                .money_ai_mut(self.owner)
                .base
                .set_emoticon(crate::ai::EmoticonType::QuestionMark);

            self.duty_set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
            self.engine
                .money_ai_mut(self.owner)
                .money_fight_enemies
                .push(thief.get());
            self.react_to_stolen_money();
            self.engine.money_ai_mut(self.owner).base.friend_in_trouble = Some(thief);
        } else if substate.is_fight_for_money() {
            self.engine
                .money_ai_mut(self.owner)
                .money_fight_enemies
                .push(thief.get());
        }
    }

    fn react_to_stolen_money(&mut self) {
        let entity = self
            .engine
            .expect_entity(self.owner, "money reaction owner");
        if self.engine.is_player_aligned_camp(entity.camp())
            && self.engine.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
        {
            self.engine.money_timer(self.owner, 3);
            return;
        }
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
        let frames = ((100.0
            - self
                .engine
                .money_ai(self.owner)
                .profile(&self.assets.profile_manager)
                .intelligence as f32)
            * 0.01
            * crate::parameters_ai::AI_MAX_ENEMY_REACTIONTIME as f32
            * modifier
            + 1.0) as u32;
        self.engine.money_timer(self.owner, frames);
    }
}
