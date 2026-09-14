//! Money-fight scans and their synchronous caller tails.

use super::*;
use crate::ai::{AiEntityHandle, AiState, DutyFlags, GotoFlags, MoneyFightOperation, Substate};
use crate::ai_enemy::EnemyAi;
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(xs: &[f32]) -> (EngineInner, LevelAssets, Vec<EntityId>) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                1,
                0,
                MapPoint::new(0.0, 0.0),
                MapPoint::new(2000.0, 2000.0),
            ),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
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
        engine.ai.global.all_soldier_handles =
            std::sync::Arc::new(ids.iter().map(|id| id.index()).collect());
        (engine, LevelAssets::new(), ids)
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
        let (mut engine, assets, ids) = fixture(&[500.0, 600.0, 700.0, 800.0, 900.0]);
        engine.money_ai_mut(ids[0]).soldier_profile_money = 40;
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
    fn sorting_keys_survive_runtime_clones_but_not_saved_frames_or_hashes() {
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
        assert_eq!(bitcode::encode(&human), binary);
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
        self.world
            .entities
            .expect_enemy_ai(owner, format_args!("money-fight owner"))
    }

    fn money_ai_mut(&mut self, owner: EntityId) -> &mut EnemyAi {
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("money-fight owner"))
    }

    fn money_camp_soldier(&self, camp: Camp, index: usize) -> Option<EntityId> {
        let id = EntityId::Soldier(crate::entity_id::SoldierId(
            self.ai.global.all_soldier_handles[index],
        ));
        self.world
            .entities
            .get(id)
            .filter(|entity| entity.soldier_data().is_some() && entity.camp() == camp)
            .map(|_| id)
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

    pub(in crate::engine) fn execute_money_fight(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        operation: MoneyFightOperation,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        match operation {
            MoneyFightOperation::CleanUpAfterBrawl => {
                self.create_live_money_fight_victims(assets, owner);
                self.approach_next_money_victim(sim, assets, owner, false);
            }
            MoneyFightOperation::CollectOrLootAfterLook => {
                self.create_live_money_fight_victims(assets, owner);
                while let Some(&handle) = self.money_ai(owner).money_fight_victims.first() {
                    let target =
                        self.expect_human_id_for_ai_handle(handle, "money-fight looted victim");
                    if !self.money_ai(target).base.looted_after_money_fight {
                        break;
                    }
                    self.money_ai_mut(owner).money_fight_victims.remove(0);
                }
                self.approach_next_money_victim(sim, assets, owner, true);
            }
            MoneyFightOperation::AwakeNextVictim => {
                self.approach_next_money_victim(sim, assets, owner, false)
            }
            MoneyFightOperation::FinishHitAfterOfficer => self.finish_money_hit(sim, assets, owner),
            MoneyFightOperation::FinishBrawl => self.finish_live_brawl(sim, assets, owner),
            MoneyFightOperation::RecoverBrawl => {
                if let Some(target) = self.nearest_money_fight_enemy(owner) {
                    self.money_ai_mut(owner).base.friend_in_trouble =
                        Some(AiEntityHandle::new(target));
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingBrawlApproaching,
                    );
                    let target = self
                        .money_ai(owner)
                        .base
                        .friend_in_trouble
                        .expect("brawl approach lost its target");
                    let target =
                        self.expect_human_id_for_ai_handle(target.get(), "brawl approach target");
                    self.duty_go_near(
                        sim,
                        assets,
                        owner,
                        self.live_ai_position(target),
                        crate::parameters_ai::AI_HIT_DISTANCE,
                        GotoFlags::RUN,
                    );
                    self.execute_maybe_officer_sees_me_fighting(sim, assets, owner);
                } else {
                    self.stop_live_brawl_and_collect_money(sim, assets, owner);
                }
            }
            MoneyFightOperation::StolenMoney { object, thief } => {
                self.handle_stolen_money(sim, assets, owner, object, thief)
            }
        }
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn create_live_money_fight_victims(&mut self, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "money victim scan camp").camp();
        self.money_ai_mut(owner).money_fight_victims.clear();
        // Distances are captured before each visibility query. Equal distances
        // insert before earlier candidates, including the owner when eligible.
        for index in 0..self.ai.global.all_soldier_handles.len() {
            let Some(target) = self.money_camp_soldier(camp, index) else {
                continue;
            };
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
            self.world
                .entities
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
        if self.money_ai(owner).soldier_profile_money == 100
            || self.money_ai(owner).base.blood_alcohol > 0
        {
            return true;
        }
        let mut upright = 1_u16;
        let mut sleeping = 0_u16;
        for index in 0..self.ai.global.all_soldier_handles.len() {
            let Some(target) = self.money_camp_soldier(camp, index) else {
                continue;
            };
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
            < u32::from(self.money_ai(owner).soldier_profile_money)
    }

    fn create_live_money_fight_enemies(&mut self, assets: &LevelAssets, owner: EntityId) {
        let camp = self.expect_entity(owner, "money enemy scan camp").camp();
        self.money_ai_mut(owner).money_fight_enemies.clear();
        for index in 0..self.ai.global.all_soldier_handles.len() {
            let Some(target) = self.money_camp_soldier(camp, index) else {
                continue;
            };
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

    fn approach_next_money_victim(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        loot: bool,
    ) {
        if self.money_ai(owner).money_fight_victims.is_empty() {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        let next = self.money_ai_mut(owner).money_fight_victims.remove(0);
        self.money_ai_mut(owner).base.detected_body = Some(AiEntityHandle::new(next));
        if loot {
            let victim = self.expect_human_id_for_ai_handle(next, "money-fight victim to loot");
            self.money_ai_mut(victim).base.looted_after_money_fight = true;
        }
        let substate = if loot {
            Substate::WonderingApproachingToLoot
        } else {
            Substate::WonderingApproachingBrawlVictim
        };
        self.duty_set_state(sim, assets, owner, AiState::Wondering, substate);
        let body = self
            .money_ai(owner)
            .base
            .detected_body
            .expect("money victim state change lost its body");
        let body = self.expect_human_id_for_ai_handle(body.get(), "money-fight victim movement");
        self.duty_go_near(
            sim,
            assets,
            owner,
            self.live_ai_position(body),
            crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
            GotoFlags::empty(),
        );
    }

    fn finish_money_hit(&mut self, sim: &SimulationContext, assets: &LevelAssets, owner: EntityId) {
        if let Some(friend) = self.money_ai(owner).base.friend_in_trouble {
            let target = self.expect_human_id_for_ai_handle(friend.get(), "brawl-hit partner");
            if self
                .expect_entity(target, "brawl-hit partner")
                .is_unconscious()
            {
                self.money_ai_mut(owner)
                    .money_fight_enemies
                    .retain(|&handle| handle != friend.get());
            }
        }
        if self.money_ai(owner).money_fight_enemies.is_empty() {
            self.create_live_money_fight_enemies(assets, owner);
        }
        if !self.wants_live_money_fight(assets, owner) {
            self.money_ai_mut(owner).money_fight_enemies.clear();
            self.stop_live_brawl_and_collect_money(sim, assets, owner);
        } else if self
            .money_ai(owner)
            .base
            .friend_in_trouble
            .is_some_and(|friend| {
                let target =
                    self.expect_human_id_for_ai_handle(friend.get(), "brawl-hit surviving partner");
                !self
                    .expect_entity(target, "brawl-hit surviving partner")
                    .is_unconscious()
            })
        {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingBrawlReactiontime,
            );
            let target = self
                .money_ai(owner)
                .base
                .friend_in_trouble
                .expect("brawl reaction lost partner");
            self.face_money_human(sim, assets, owner, target);
            self.money_timer(owner, 30);
        } else if let Some(target) = self.nearest_money_fight_enemy(owner) {
            self.money_ai_mut(owner).base.friend_in_trouble = Some(AiEntityHandle::new(target));
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingBrawlReactiontime,
            );
            self.money_timer(owner, 10);
        } else {
            self.stop_live_brawl_and_collect_money(sim, assets, owner);
        }
    }

    fn money_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.money_ai_mut(owner).base.launch_timer(frames, frame);
    }

    fn finish_live_brawl(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Wondering,
            Substate::WonderingOfficerFinishingBrawl,
        );
        let camp = self.expect_entity(owner, "finish brawl scan camp").camp();
        assert_eq!(
            self.money_ai(owner).get_rank(),
            crate::profiles::ProfileRank::Officer
        );
        self.money_ai_mut(owner).base.antagonist = None;
        self.money_ai_mut(owner).base.list_us.clear();
        for index in 0..self.ai.global.all_soldier_handles.len() {
            let Some(target) = self.money_camp_soldier(camp, index) else {
                continue;
            };
            let ai = self.money_ai(target);
            if ai.get_rank() != crate::profiles::ProfileRank::Soldier
                || !(ai.base.current_substate.is_take_money()
                    || ai.base.current_substate.is_fight_for_money())
                || !self.patrol_member_visible(assets, target, owner)
            {
                continue;
            }
            self.execute_ai_callback(
                sim,
                assets,
                target,
                &crate::ai::Stimulus::with_human(StimulusType::CallFinishBrawl, owner.index()),
            );
            let ai = self.money_ai_mut(owner);
            ai.base.list_us.push(target.index());
            if ai.base.antagonist.is_none() {
                ai.base.antagonist = Some(AiEntityHandle::new(target.index()));
            }
        }
        if let Some(target) = self.money_ai(owner).base.antagonist {
            self.face_money_human(sim, assets, owner, target);
            self.owner_work_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::OfficerEndsBrawl,
                    flags: crate::ai::SpeechFlags::MYTALK_1.bits(),
                },
            );
        }
        self.money_ai_mut(owner)
            .base
            .set_emoticon(crate::ai::EmoticonType::Thunderstorm);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.money_timer(owner, 200);
    }

    fn face_money_human(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: AiEntityHandle,
    ) {
        let target = self.expect_human_id_for_ai_handle(target.get(), "money-fight facing target");
        let elevation = self
            .expect_entity(target, "money-fight facing target")
            .position_iface()
            .get_elevation();
        self.duty_face_position_at_elevation(
            sim,
            assets,
            owner,
            self.live_ai_position(target),
            elevation,
        );
    }

    fn stop_live_brawl_and_collect_money(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let coin = self.take_nearest_live_money(owner);
        self.money_ai_mut(owner).base.interesting_object = coin.map(AiEntityHandle::new);
        if coin.is_some() {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingRunningForMoney,
            );
            let target = self
                .money_ai(owner)
                .base
                .interesting_object
                .expect("money movement lost coin");
            let target = self.expect_entity_id_for_index(target.get(), "money movement coin");
            self.duty_go_near(
                sim,
                assets,
                owner,
                self.live_ai_position(target),
                crate::parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                GotoFlags::RUN | GotoFlags::FIND_ACCESSIBLE,
            );
        } else {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingWatchingForMoreMoney,
            );
            self.money_ai_mut(owner).base.outbox.actor.look_sidewards =
                Some(crate::ai::LookDirection::LeftRight);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
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

    fn handle_stolen_money(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        object: AiEntityHandle,
        thief: AiEntityHandle,
    ) {
        let object_id = self.expect_entity_id_for_index(object.get(), "stolen object");
        let object_type = self
            .expect_entity(object_id, "stolen object")
            .object_data()
            .expect("stolen item is not an object")
            .object_type;
        if !matches!(
            object_type,
            crate::element::ObjectType::Coin | crate::element::ObjectType::Purse
        ) {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
            return;
        }
        let target = self.expect_human_id_for_ai_handle(thief.get(), "coin thief");
        if !self.live_ai_detects_180(assets, owner, target) {
            return;
        }
        assert_ne!(owner, target, "coin thief cannot be the owner");
        let ai = self.money_ai(owner);
        if ai.base.interesting_object != Some(object)
            && !ai.other_seen_money.contains(&object.get())
        {
            return;
        }
        let entity = self.expect_entity(owner, "stolen money owner");
        let wants_money = ai.base.blood_alcohol as i32
            > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            || (entity.is_active()
                && self
                    .entity_building_sector(entity.element_data().sector())
                    .is_none()
                && ai.soldier_profile_money > 0);
        if !wants_money {
            return;
        }
        if !self.wants_live_money_fight(assets, owner) {
            self.money_ai_mut(owner).money_fight_enemies.clear();
            self.stop_live_brawl_and_collect_money(sim, assets, owner);
            return;
        }
        let substate = self.money_ai(owner).base.current_substate;
        if substate.is_take_money() {
            self.money_ai_mut(owner).base.break_macro();
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.face_money_human(sim, assets, owner, thief);
            self.money_ai_mut(owner)
                .base
                .set_emoticon(crate::ai::EmoticonType::QuestionMark);
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Wondering,
                Substate::WonderingBrawlReactiontime,
            );
            self.money_ai_mut(owner)
                .money_fight_enemies
                .push(thief.get());
            self.react_to_stolen_money(sim, owner);
            self.money_ai_mut(owner).base.friend_in_trouble = Some(thief);
        } else if substate.is_fight_for_money() {
            self.money_ai_mut(owner)
                .money_fight_enemies
                .push(thief.get());
        }
    }

    fn react_to_stolen_money(&mut self, sim: &SimulationContext, owner: EntityId) {
        let entity = self.expect_entity(owner, "money reaction owner");
        if self.is_player_aligned_camp(entity.camp())
            && self.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider)
        {
            self.money_timer(owner, 3);
            return;
        }
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
        let frames = ((100.0 - self.money_ai(owner).soldier_profile_iq as f32)
            * 0.01
            * crate::parameters_ai::AI_MAX_ENEMY_REACTIONTIME as f32
            * modifier
            + 1.0) as u32;
        self.money_timer(owner, frames);
    }
}
