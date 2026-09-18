//! Live enemy approach, reciprocal target exchange, and rider charges.

use super::swordfight_candidates::LiveCombatFighters;
use super::*;
use crate::ai::{AiEntityHandle, AiSpeechAttempt, AiState, GotoFlags, Position, Remark, Substate};
use crate::ai_enemy::{AiMapVec, CombatFighterAccess, rider_charge_goal_geometry};
use crate::engine::TickCtx;
use crate::sim_rng::SimulationContext;
use crate::weapons::WeaponDistance;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_rebalance_swordfight(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        target: EntityId,
    ) {
        if self.direct_enter_swordfight(tcx, owner, target) {
            self.enemy_ai_mut(owner, "combat rebalance")
                .base
                .primary_target = Some(AiEntityHandle::new(target.index()));
        }
    }

    pub(in crate::engine) fn set_live_guarded_pc(
        &mut self,
        owner: EntityId,
        new_pc: Option<crate::entity_id::PcId>,
    ) {
        let old_pc = self.enemy_ai(owner, "guard owner").guarded_pc;
        if let Some(old_pc) = old_pc {
            self.entities_mut()
                .get_mut(EntityId::Pc(old_pc))
                .expect("previous guarded PC exists")
                .pc_data_mut()
                .expect("guarded actor is a PC")
                .guard = None;
        }
        self.enemy_ai_mut(owner, "guard owner").guarded_pc = new_pc;
        if let Some(new_pc) = new_pc {
            self.entities_mut()
                .get_mut(EntityId::Pc(new_pc))
                .expect("new guarded PC exists")
                .pc_data_mut()
                .expect("guarded actor is a PC")
                .guard = Some(owner);
        }
    }

    pub(in crate::engine) fn clear_live_combat_neighbours(&mut self, owner: EntityId) {
        let left = self
            .enemy_ai(owner, "clear left combat neighbour")
            .left_combat_neighbour;
        self.apply_update_left_combat_neighbour(owner.index(), left, None);
        let right = self
            .enemy_ai(owner, "clear right combat neighbour")
            .right_combat_neighbour;
        self.apply_update_right_combat_neighbour(owner.index(), right, None);
    }

    fn approach_primary(&self, owner: EntityId) -> EntityId {
        let handle = self
            .ai(owner, "approach primary")
            .primary_target
            .expect("enemy approach requires a primary target");
        self.expect_human_id_for_ai_handle(handle.get(), "approach primary")
    }

    fn approach_timer(&mut self, owner: EntityId, delay: u32) {
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("approach timer"))
            .launch_timer(delay, self.control.frame_counter);
    }

    fn approach_focus(&mut self, owner: EntityId, target: Option<EntityId>) {
        self.execute_ai_focus(
            owner,
            target.map(|target| AiEntityHandle::new(target.index())),
        );
    }

    fn approach_sword_range(&self, assets: &LevelAssets, owner: EntityId) -> u16 {
        LiveCombatFighters {
            engine: self,
            assets,
            owner,
        }
        .range(owner.index(), WeaponDistance::Default)
    }

    pub(in crate::engine) fn execute_ai_begin_swordfight(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_begin_swordfight()
    }

    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_reconsider_enemy_approach(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        reachpoint: bool,
    ) {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_reconsider_enemy_approach(reachpoint)
    }

    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_maybe_make_rider_attack(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) -> bool {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_maybe_make_rider_attack()
    }

    fn live_rider_attack_destination(
        &self,
        owner: EntityId,
        target: EntityId,
        position: Position,
        direction: u16,
    ) -> Option<(Position, bool)> {
        let raw = self
            .expect_entity(target, "rider geometry target")
            .element_data()
            .position_map();
        let geometry =
            rider_charge_goal_geometry((position.x, position.y), direction, (raw.x, raw.y)).ok()?;
        let goal = MapPoint::new(geometry.goal.0, geometry.goal.1);
        if !self.world.fast_grid.is_straight_movement_authorized(
            position.map_point(),
            goal,
            position.level,
            self.expect_entity(owner, "rider movement box")
                .position_iface()
                .get_move_box(),
        ) {
            return None;
        }
        let mut before = geometry.hit_norm_len;
        while before > 80.0 {
            before -= 80.0;
        }
        let (dx, dy) = geometry.hit_dir;
        let normal = (
            -dy * crate::position_interface::INVERSE_ASPECT_RATIO,
            dx * crate::position_interface::ASPECT_RATIO,
        );
        let first = (
            position.x + geometry.me_to_hit.0 - dx * before,
            position.y + geometry.me_to_hit.1 - dy * before,
        );
        let points = [
            first,
            (first.0 + dx * 80.0, first.1 + dy * 80.0),
            (
                first.0 + dx * 80.0 + normal.0 * 65.0,
                first.1 + dy * 80.0 + normal.1 * 65.0,
            ),
            (first.0 + normal.0 * 65.0, first.1 + normal.1 * 65.0),
            first,
        ];
        let polygon = geo::Polygon::new(
            geo::LineString::from(
                points
                    .into_iter()
                    .map(|(x, y)| (f64::from(x), f64::from(y)))
                    .collect::<Vec<_>>(),
            ),
            vec![],
        );
        let camp = self.expect_entity(owner, "rider corridor camp").camp();
        use geo::Contains;
        let fighter_count = self.world.fighter_registry_ids.len();
        for index in 0..fighter_count {
            let friend = self.world.fighter_registry_ids[index];
            let entity = self.expect_entity(friend, "rider corridor friend");
            if friend == owner
                || entity.camp() != camp
                || entity.is_dead()
                || entity.is_unconscious()
                || entity.element_data().layer() != position.level
            {
                continue;
            }
            let point = entity.element_data().position_map();
            if polygon.contains(&geo::Point::new(f64::from(point.x), f64::from(point.y))) {
                return None;
            }
        }
        Some((
            Position {
                x: goal.x,
                y: goal.y,
                sector: position.sector,
                level: position.level,
            },
            geometry.me_to_hit.0 * geometry.me_to_hit.0
                + geometry.me_to_hit.1 * geometry.me_to_hit.1
                < 6400.0,
        ))
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_end_swordfight(&mut self) {
        if self
            .engine
            .expect_entity(self.owner, "end swordfight owner")
            .human_data()
            .expect("swordfighter is human")
            .opponents
            .is_empty()
        {
            return;
        }
        self.engine.launch_element(
            TickCtx::new(self.sim, self.assets),
            crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(self.owner),
            ),
        );
    }

    pub(in crate::engine) fn launch_ai_raise_sword(&mut self) {
        let mut element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::EnterSwordfight,
            Some(self.owner),
        );
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::JumplineDestination,
            crate::sequence::FieldValue::Integer(0),
        );
        self.engine
            .launch_element(TickCtx::new(self.sim, self.assets), element);
    }

    pub(in crate::engine) fn execute_ai_begin_swordfight(&mut self) {
        self.stop_ai_owner();
        self.engine
            .nearby_civilians_panic(TickCtx::new(self.sim, self.assets), self.owner);

        let target = self.engine.approach_primary(self.owner);
        let entity = self.engine.expect_entity(target, "swordfight target stop");
        if entity
            .human_data()
            .expect("swordfight target is human")
            .opponents
            .is_empty()
            && entity
                .actor_data()
                .expect("swordfight target is actor")
                .action_state
                .is_moving()
        {
            self.engine.stop_actor_orders(
                TickCtx::new(self.sim, self.assets),
                &mut Vec::new(),
                target,
                crate::sequence::SequencePriority::Normal,
            );
        }
        let ai = self.engine.enemy_ai(self.owner, "swordfight entry");
        let target = self.engine.expect_human_id_for_ai_handle(
            ai.base.primary_target.expect("swordfight primary").get(),
            "swordfight entry target",
        );
        let jump_line = ai
            .my_line_jump
            .and_then(crate::jump_line::JumpLineIndex::new);
        let mut element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::EnterSwordfight,
            Some(self.owner),
        );
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Element(target),
        );
        element.set_property(
            crate::sequence::Field::JumplineDestination,
            jump_line
                .map(crate::sequence::FieldValue::LineId)
                .unwrap_or(crate::sequence::FieldValue::Integer(0)),
        );
        element.set_property(
            crate::sequence::Field::SwordfightPrepared,
            crate::sequence::FieldValue::Integer(0),
        );
        self.engine
            .launch_element(TickCtx::new(self.sim, self.assets), element);

        self.engine.clear_live_combat_neighbours(self.owner);
        self.engine.approach_focus(self.owner, None);
        let vip = self
            .engine
            .expect_entity(self.owner, "swordfight speech")
            .is_vip();
        self.execute_ai_speech(AiSpeechAttempt {
            remark: if vip {
                Remark::VipStartsCombat
            } else {
                Remark::StartsCombat
            },
            flags: 0,
        });
        self.engine
            .ai_mut(self.owner, "swordfight emoticon")
            .clear_emoticon();
        self.duty_set_state(AiState::Attacking, Substate::AttackingSwordfight);
        self.engine.approach_timer(self.owner, 20);
    }

    pub(in crate::engine) fn execute_ai_reconsider_enemy_approach(&mut self, reachpoint: bool) {
        if !self
            .engine
            .expect_entity(self.owner, "approach combat gate")
            .human_data()
            .expect("approach owner is human")
            .opponents
            .is_empty()
        {
            self.duty_set_state(AiState::Attacking, Substate::AttackingSwordfight);
            self.engine.approach_timer(self.owner, 30);
            return;
        }
        if self.refresh_ai_arrow_protection(false) {
            return;
        }
        let mut target = self.engine.approach_primary(self.owner);
        let entity = self.engine.expect_entity(target, "approach carried target");
        if entity.element_data().posture() == crate::element::Posture::OnShoulders {
            target = entity
                .human_data()
                .expect("carried target is human")
                .carrier
                .expect("shoulder target requires carrier");
            self.engine
                .ai_mut(self.owner, "approach carrier substitution")
                .primary_target = Some(AiEntityHandle::new(target.index()));
        }
        let standard_range = self.engine.approach_sword_range(self.assets, self.owner);
        let sword_range = standard_range.wrapping_add(10);
        let courage = self
            .engine
            .enemy_ai(self.owner, "approach courage")
            .get_courage(&self.assets.profile_manager);
        let mut run_distance = (2 * (100 - courage)).max(sword_range);
        let my_position = self.engine.live_ai_position(self.owner);
        let mut target_position = self.engine.live_ai_position(target);
        let mut distance = (target_position.map_point() - my_position.map_point())
            .square_norm()
            .sqrt() as u16;
        let maximal_range = LiveCombatFighters {
            engine: self.engine,
            assets: self.assets,
            owner: self.owner,
        }
        .sword_range_maximal(self.owner.index());
        let owner_element = self
            .engine
            .expect_entity(self.owner, "approach table owner")
            .element_data();
        let target_element = self
            .engine
            .expect_entity(target, "approach table target")
            .element_data();
        let line = crate::engine::melee::table_swordfight_jump_line(
            &self.engine.world.fast_grid,
            owner_element.sector().map(i16::from).unwrap_or(-1),
            target_element.sector().map(i16::from).unwrap_or(-1),
            target_element.position_map(),
            f32::from(maximal_range),
        );
        self.engine
            .enemy_ai_mut(self.owner, "approach jump line")
            .my_line_jump = line;
        let camp = self
            .engine
            .expect_entity(self.owner, "approach friend camp")
            .camp();
        // Every successful exchange changes the next comparison's primary target.
        let fighter_count = self.engine.world.fighter_registry_ids.len();
        for index in 0..fighter_count {
            let friend = self.engine.world.fighter_registry_ids[index];
            if friend == self.owner
                || !matches!(
                    self.engine.expect_entity(friend, "approach friend kind"),
                    Entity::Soldier(_)
                )
                || self
                    .engine
                    .expect_entity(friend, "approach friend camp")
                    .camp()
                    != camp
            {
                continue;
            }
            let ai = self.engine.enemy_ai(friend, "approach friend");
            if ai.base.primary_target == Some(AiEntityHandle::new(target.index()))
                || !matches!(
                    ai.base.current_substate,
                    Substate::AttackingRunningToEnemy
                        | Substate::AttackingWalkingToEnemy
                        | Substate::AttackingChargingEnemy
                )
            {
                continue;
            }
            let friend_target = self.engine.approach_primary(friend);
            let friend_position = self.engine.live_ai_position(friend);
            let friend_target_position = self.engine.live_ai_position(friend_target);
            let to_friend_target = (my_position.map_point() - friend_target_position.map_point())
                .square_norm()
                .sqrt();
            if to_friend_target
                + (friend_position.map_point() - target_position.map_point())
                    .square_norm()
                    .sqrt()
                < f32::from(distance)
                    + (friend_position.map_point() - friend_target_position.map_point())
                        .square_norm()
                        .sqrt()
            {
                self.engine
                    .ai_mut(friend, "approach exchange friend")
                    .primary_target = Some(AiEntityHandle::new(target.index()));
                target = friend_target;
                self.engine
                    .ai_mut(self.owner, "approach exchange owner")
                    .primary_target = Some(AiEntityHandle::new(target.index()));
                distance = to_friend_target as u16;
                target_position = friend_target_position;
            }
        }

        let layer = self
            .engine
            .expect_entity(self.owner, "approach lift layer")
            .element_data()
            .layer();
        if let Some(Some(entry)) = crate::ai::enemy_lift_approach_for_position(
            &self.engine.world.fast_grid,
            target_position,
            Some(layer),
        ) {
            self.duty_set_state(AiState::Attacking, Substate::AttackingRunningToLadder);
            self.engine
                .approach_focus(self.owner, Some(self.engine.approach_primary(self.owner)));
            self.duty_go_near(entry, 30, GotoFlags::RUN);
            self.engine.approach_timer(self.owner, 30);
            return;
        }
        let lift = target_position.sector.is_some_and(|sector| {
            self.engine
                .world
                .fast_grid
                .sector_type_for_handle(sector)
                .is_lift()
        });
        let ai = self.engine.enemy_ai(self.owner, "approach charge state");
        let mut reconsider = false;
        let (mut charge, first) = match ai.base.current_substate {
            Substate::AttackingRunningToEnemy | Substate::AttackingWalkingToEnemy => (false, false),
            Substate::AttackingChargingEnemy => {
                reconsider = ai.my_line_jump.is_some() || lift;
                (!reconsider, false)
            }
            Substate::AttackingReactiontime | Substate::AttackingReactiontimeRunning => (
                ai.sword_is_charge_weapon
                    && ai.get_courage(&self.assets.profile_manager)
                        >= crate::ai_enemy::combat::CHARGE_MIN_COURAGE
                    && i32::from(distance) >= crate::ai_enemy::combat::CHARGE_MIN_DISTANCE
                    && ai.my_line_jump.is_none()
                    && !self
                        .engine
                        .expect_entity(self.owner, "approach rider")
                        .soldier_data()
                        .unwrap()
                        .rider
                    && !lift,
                true,
            ),
            _ => (false, true),
        };
        if charge && first {
            self.execute_ai_speech(AiSpeechAttempt {
                remark: Remark::Warcry,
                flags: 0,
            });
        }
        self.engine
            .approach_focus(self.owner, Some(self.engine.approach_primary(self.owner)));
        if self
            .engine
            .expect_entity(self.owner, "approach rider")
            .soldier_data()
            .unwrap()
            .rider
            && self.execute_ai_maybe_make_rider_attack()
        {
            return;
        }
        if distance <= sword_range && (!charge || reachpoint) {
            self.execute_ai_begin_swordfight();
            return;
        }
        reconsider |= reachpoint || first;
        let position = self
            .engine
            .live_ai_position(self.engine.approach_primary(self.owner));
        let ai = self
            .engine
            .enemy_ai(self.owner, "approach reconsider gates");
        reconsider |= (position.map_point() - ai.base.seek_position.map_point()).square_norm()
            > if charge { 10.0 } else { 100.0 }
            && !ai.pc_missed;
        let mut below = !charge && distance < run_distance.wrapping_add(10);
        reconsider |=
            !charge && ai.base.current_substate == Substate::AttackingRunningToEnemy && below;
        below &= !self
            .engine
            .expect_entity(self.owner, "approach rider")
            .soldier_data()
            .unwrap()
            .rider;
        if self
            .engine
            .actor_command(self.engine.approach_primary(self.owner)) as u16
            != crate::order::OrderType::WalkingCarryingOnShoulders as u16
        {
            charge = false;
            below = false;
            reconsider = true;
            run_distance = self.engine.approach_sword_range(self.assets, self.owner);
        }
        if !reconsider {
            self.engine.approach_timer(self.owner, 10);
            return;
        }
        target = self.engine.approach_primary(self.owner);
        let line = self
            .engine
            .enemy_ai(self.owner, "approach goal line")
            .my_line_jump;
        let goal = if let Some(line) = line {
            let raw = self
                .engine
                .expect_entity(target, "approach jump target")
                .element_data()
                .position_map();
            self.engine
                .enemy_ai(self.owner, "approach jump goal")
                .compute_jump_line_target(
                    &self.engine.world.fast_grid,
                    line,
                    Position {
                        x: raw.x,
                        y: raw.y,
                        ..target_position
                    },
                )
                .expect("approach jump line requires associated geometry")
        } else {
            self.engine.live_ai_position(target)
        };
        self.engine
            .ai_mut(self.owner, "approach seek goal")
            .seek_position = goal;
        self.engine
            .approach_focus(self.owner, Some(self.engine.approach_primary(self.owner)));
        let (state, flags, tolerance) = if !below {
            if charge {
                (
                    Substate::AttackingChargingEnemy,
                    GotoFlags::RUN | GotoFlags::CHARGE,
                    self.engine.approach_sword_range(self.assets, self.owner),
                )
            } else {
                (
                    Substate::AttackingRunningToEnemy,
                    GotoFlags::RUN | GotoFlags::DONT_STOP,
                    run_distance,
                )
            }
        } else if self
            .engine
            .live_actor_animation(self.engine.approach_primary(self.owner))
            == Some(crate::order::OrderType::RunningUpright)
        {
            (
                Substate::AttackingRunningToEnemy,
                GotoFlags::RUN | GotoFlags::DONT_STOP,
                self.engine.approach_sword_range(self.assets, self.owner),
            )
        } else {
            (
                Substate::AttackingWalkingToEnemy,
                GotoFlags::empty(),
                self.engine.approach_sword_range(self.assets, self.owner),
            )
        };
        if below && line.is_some() {
            self.duty_go_to(goal, flags);
        } else {
            self.duty_go_near(goal, i32::from(tolerance), flags);
        }
        let ai = self.engine.ai_mut(self.owner, "approach completion");
        if ai.already_on_point {
            ai.already_on_point = false;
            self.execute_ai_begin_swordfight();
            return;
        }
        self.duty_set_state(AiState::Attacking, state);
        self.engine.approach_timer(self.owner, 10);
        if self
            .engine
            .ai(self.owner, "approach route result")
            .couldnt_reachpoint
        {
            let target = self.engine.approach_primary(self.owner);
            let wait = precompute_avenger_on_roof_wait_position(
                &self.engine.entities(),
                self.engine.script_domains.interactables.doors.as_slice(),
                &self.engine.seq(),
                self.owner,
                target,
                |element| super::ai_view_position_sector(self.engine, element),
                &|sector| self.engine.building_sector_is_authorized(sector),
                &|sector| self.engine.get_sector_lift_type(sector),
            );
            if let Some(wait) = wait {
                self.engine
                    .ai_mut(self.owner, "roof route result")
                    .couldnt_reachpoint = false;
                self.duty_set_state(AiState::Attacking, Substate::AttackingRunToAvengerOnRoof);
                self.duty_go_near(wait, 50, GotoFlags::RUN);
                let position = self
                    .engine
                    .live_ai_position(self.engine.approach_primary(self.owner));
                self.engine.ai_mut(self.owner, "roof target").seek_position = position;
            }
        }
    }

    pub(in crate::engine) fn execute_ai_maybe_make_rider_attack(&mut self) -> bool {
        assert!(
            self.engine
                .expect_entity(self.owner, "rider attack")
                .soldier_data()
                .expect("rider is soldier")
                .rider
        );
        let element = self
            .engine
            .expect_entity(self.owner, "rider raw position")
            .element_data();
        let raw = element.position_map();
        let position = Position {
            x: raw.x,
            y: raw.y,
            sector: element.sector(),
            level: element.layer(),
        };
        let direction = self
            .engine
            .expect_entity(self.owner, "rider direction")
            .element_data()
            .direction() as u16;
        let primary = self.engine.ai(self.owner, "rider primary").primary_target;
        let mut goal = primary.and_then(|handle| {
            let target = self
                .engine
                .expect_human_id_for_ai_handle(handle.get(), "rider primary");
            let entity = self.engine.expect_entity(target, "rider primary gates");
            (!entity.is_dead()
                && !entity.is_unconscious()
                && entity.element_data().posture() != crate::element::Posture::Tied)
                .then(|| {
                    self.engine
                        .live_rider_attack_destination(self.owner, target, position, direction)
                })
                .flatten()
        });
        if goal.is_none() {
            let count = self
                .engine
                .enemy_ai(self.owner, "rider enemy count")
                .list_them
                .len();
            for index in 0..count {
                let handle = self.engine.enemy_ai(self.owner, "rider enemy").list_them[index];
                if primary == Some(AiEntityHandle::new(handle)) {
                    continue;
                }
                let target = self
                    .engine
                    .expect_human_id_for_ai_handle(handle, "rider fallback target");
                if let Some(found) = self
                    .engine
                    .live_rider_attack_destination(self.owner, target, position, direction)
                {
                    self.engine
                        .ai_mut(self.owner, "rider chosen target")
                        .primary_target = Some(AiEntityHandle::new(handle));
                    goal = Some(found);
                    break;
                }
            }
        }
        let Some((goal, hit)) = goal else {
            return false;
        };
        let target = self.engine.approach_primary(self.owner);
        let position = self.engine.live_ai_position(target);
        self.engine
            .ai_mut(self.owner, "rider seek target")
            .seek_position = position;
        self.engine.approach_focus(self.owner, Some(target));
        let mut flags = GotoFlags::RUN | GotoFlags::RIDER_CHARGE;
        let state = if hit {
            self.engine.approach_focus(self.owner, None);
            self.execute_ai_speech(AiSpeechAttempt {
                remark: Remark::Warcry,
                flags: 0,
            });
            flags |= GotoFlags::RIDER_CHARGE_HIT;
            Substate::AttackingRiderChargingPassing
        } else {
            Substate::AttackingRiderChargingApproaching
        };
        self.duty_set_state(AiState::Attacking, state);
        self.duty_go_to(goal, flags);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_lift_target(
        engine: &mut EngineInner,
        owner: EntityId,
        target: EntityId,
    ) -> Position {
        use crate::fast_find_grid::DoorProjectionInfo;
        use crate::gate::{Door, DoorIndex};
        use crate::sector::{LiftType, SectorNumber, SectorType};
        use crate::sequence::{SequenceElement, SequenceElementData};
        let lower_sector = engine
            .expect_entity(owner, "lower ladder entry fixture")
            .element_data()
            .sector()
            .unwrap();
        engine.world.fast_grid_mut().allocate_layers(4);
        let entry_index = engine.world.fast_grid_mut().add_sector(
            square_sector(7, 3, MapPoint::new(0.0, 0.0), MapPoint::new(1000.0, 1000.0)),
            3,
        );
        let entry_sector = crate::position_interface::SectorHandle::new(7)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(entry_index).unwrap());
        let mut lift = square_sector(
            42,
            0,
            MapPoint::new(400.0, 400.0),
            MapPoint::new(600.0, 600.0),
        );
        lift.sector_type |= SectorType::LIFT;
        lift.lift_type = Some(LiftType::Ladder);
        lift.gate_indices = vec![DoorIndex::new(0).unwrap(), DoorIndex::new(1).unwrap()];
        let lift_index = engine.world.fast_grid_mut().add_sector(lift, 0);
        let lift_sector = crate::position_interface::SectorHandle::new(42)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(lift_index).unwrap());
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .door_projection_infos = vec![
            DoorProjectionInfo {
                point_out: MapPoint::new(410.0, 120.0),
                sector_out: SectorNumber::new(7),
                sector_out_index: entry_sector.arena_index(),
                layer_out: 3,
                ..Default::default()
            },
            DoorProjectionInfo {
                point_out: MapPoint::new(410.0, 800.0),
                sector_out: SectorNumber::new(1),
                sector_out_index: lower_sector.arena_index(),
                layer_out: 0,
                ..Default::default()
            },
        ];
        engine.script_domains.interactables.doors = vec![Door {
            active: true,
            sector_out: SectorNumber::new(7),
            sector_in: SectorNumber::new(42),
            sector_out_index: entry_sector.arena_index(),
            sector_in_index: lift_sector.arena_index(),
            point_out: MapPoint::new(410.0, 120.0),
            point_in: MapPoint::new(500.0, 500.0),
            layer_out: 3,
            layer_in: 0,
            ..Default::default()
        }];
        let mut lower_door = engine.script_domains.interactables.doors[0].clone();
        lower_door.point_out = MapPoint::new(410.0, 800.0);
        lower_door.sector_out = SectorNumber::new(1);
        lower_door.sector_out_index = lower_sector.arena_index();
        lower_door.layer_out = 0;
        engine.script_domains.interactables.doors.push(lower_door);
        engine.elem_mut(owner).set_sector(Some(entry_sector));
        engine.elem_mut(owner).set_layer(3);
        engine.elem_mut(target).set_sector(Some(entry_sector));
        engine.elem_mut(target).set_layer(3);
        move_actor(engine, owner, 100.0, 100.0);
        move_actor(engine, target, 70.0, 80.0);
        let mut pass = SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(target),
            crate::order::OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            unreachable!()
        };
        *gate_id = Some(DoorIndex::new(0).unwrap());
        *direction = 1;
        let id = engine.orders.sequence_manager.insert_element(pass);
        engine.orders.sequence_manager.start_sequence_level(id);
        engine.select_sequence_element(target, Some((id, 0)));
        engine.t_element_in_progress(&LevelAssets::new(), id, 0);
        Position {
            x: 410.0,
            y: 120.0,
            sector: Some(entry_sector),
            level: 3,
        }
    }

    #[test]
    fn reconsider_approach_uses_selected_door_lift_and_live_weapon_range_after_retarget() {
        let (mut engine, assets, owner, _, target) = prepare_approach(65);
        engine.control.frame_counter = 1058;
        engine.enemy_mut(owner).base.current_substate = Substate::AttackingQuittingSwordfight;
        let entry = install_lift_target(&mut engine, owner, target);
        assert_eq!(
            engine.live_ai_position(target).map_point(),
            MapPoint::new(500.0, 500.0)
        );
        assert_eq!(engine.map_pos_of(target), MapPoint::new(70.0, 80.0));
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToLadder);
        assert_eq!(ai.base.last_goto_destination, entry);
        assert_eq!(ai.base.stop_before_end_of_path_distance, 30);
        assert_eq!(ai.base.when_does_timer_ring, 1088);
        assert_eq!(pending_moves(&engine, owner).len(), 1);

        let replacement = engine.add_test_entity(make_test_pc(Posture::Upright));
        engine.elem_mut(replacement).set_sector(entry.sector);
        engine.elem_mut(replacement).set_layer(entry.level);
        move_actor(&mut engine, replacement, 700.0, 100.0);
        engine.enemy_mut(owner).base.primary_target =
            Some(AiEntityHandle::new(replacement.index()));
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            true,
        );
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
        assert_eq!(
            ai.base.last_goto_destination.map_point(),
            MapPoint::new(700.0, 100.0)
        );
        assert_eq!(ai.base.last_goto_destination.sector, entry.sector);
        assert_eq!(
            ai.base.stop_before_end_of_path_distance, 65,
            "the approach rereads the weapon profile rather than its obsolete cached range"
        );
    }

    fn roof_fixture(
        close: bool,
        waiting: bool,
    ) -> (EngineInner, LevelAssets, EntityId, EntityId, Position) {
        use crate::gate::Door;
        use crate::sector::SectorNumber;
        let (mut engine, assets, owner, _, target) = prepare_approach(50);
        let grid = engine.world.fast_grid_mut();
        grid.level_mut().sectors.clear();
        grid.level_mut().sector_number_map.clear();
        grid.size_map(128, 128);
        grid.allocate_layers(1);
        let outside_index = grid.add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(1000.0, 350.0)),
            0,
        );
        let inside_index = grid.add_sector(
            square_sector(
                2,
                0,
                MapPoint::new(0.0, 400.0),
                MapPoint::new(1000.0, 1000.0),
            ),
            0,
        );
        let outside = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(outside_index).unwrap());
        let inside = crate::position_interface::SectorHandle::new(2)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(inside_index).unwrap());
        let mut door = Door {
            active: true,
            sector_out: SectorNumber::new(1),
            sector_in: SectorNumber::new(2),
            sector_out_index: outside.arena_index(),
            sector_in_index: inside.arena_index(),
            point_out: MapPoint::new(250.0, 300.0),
            point_in: MapPoint::new(250.0, 450.0),
            ..Default::default()
        };
        door.lock_npc_villain();
        engine.script_domains.interactables.doors = vec![door];
        engine.elem_mut(owner).set_sector(Some(outside));
        engine.elem_mut(target).set_sector(Some(inside));
        move_actor(&mut engine, owner, 250.0, if close { 300.0 } else { 100.0 });
        move_actor(&mut engine, target, 264.0, 700.0);
        if waiting {
            let mut element = crate::sequence::SequenceElement::new_movement(
                1,
                crate::element::Command::MoveWaiting,
                Some(owner),
                crate::order::OrderType::RunningUpright,
            );
            // Path waiting retains the priority assigned before Move was translated.
            element.priority = crate::sequence::SequencePriority::Normal;
            let id = engine.orders.sequence_manager.insert_element(element);
            engine.orders.sequence_manager.start_sequence_level(id);
            engine.select_sequence_element(owner, Some((id, 0)));
            engine.t_element_in_progress(&LevelAssets::new(), id, 0);
        }
        (
            engine,
            assets,
            owner,
            target,
            Position {
                x: 250.0,
                y: 300.0,
                sector: Some(outside),
                level: 0,
            },
        )
    }

    #[test]
    fn failed_approach_executes_roof_wait_and_preserves_close_and_path_waiter_cases() {
        for (close, waiting) in [(false, false), (true, true), (false, true)] {
            let (mut engine, assets, owner, target, wait) = roof_fixture(close, waiting);
            let selected_waiter = waiting.then(|| {
                engine
                    .world
                    .entities
                    .current_element_for_actor(owner)
                    .expect("fixture has a selected path waiter")
                    .0
            });
            let sequences_before = engine.orders.sequence_manager.sequence_count();
            let target_position = engine.live_ai_position(target);
            engine.execute_ai_reconsider_enemy_approach(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner,
                true,
            );
            let ai = engine.enemy(owner);
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingRunToAvengerOnRoof
            );
            assert_eq!(ai.base.last_goto_destination, wait);
            assert_eq!(ai.base.seek_position, target_position);
            assert!(!ai.base.couldnt_reachpoint);
            assert_eq!(ai.base.stop_before_end_of_path_distance, 50);
            if close {
                assert!(ai.base.already_on_point);
                let selected = engine
                    .world
                    .entities
                    .current_element_for_actor(owner)
                    .expect("already-near movement retains the current waiter");
                assert_eq!(
                    Some(selected.0),
                    selected_waiter,
                    "the already-near shortcut retains the selected path waiter"
                );
                assert_eq!(
                    engine
                        .orders
                        .sequence_manager
                        .get_element(selected.0, selected.1)
                        .unwrap()
                        .command,
                    crate::element::Command::MoveWaiting,
                );
            } else if waiting {
                assert!(engine.orders.sequence_manager.sequence_count() > sequences_before);
                assert!(
                    pending_moves(&engine, owner)
                        .iter()
                        .all(|(sequence, _)| Some(*sequence) == selected_waiter),
                    "the fallback is halted after registration; the old waiter's stop transition may remain"
                );
            } else {
                assert_eq!(pending_moves(&engine, owner).len(), 1);
            }
            assert_eq!(engine.ai_think_depth(), 1);
        }
    }

    #[test]
    fn reconsider_approach_uses_raw_truncated_map_distance_at_sword_range() {
        let (mut engine, assets, owner, friend, target) = prepare_approach(62);
        move_actor(&mut engine, owner, 655.007_8, 1744.445);
        move_actor(&mut engine, target, 585.0, 1726.0);
        engine.set_active(friend, false);
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        assert_eq!(
            engine.enemy(owner).base.current_substate,
            Substate::AttackingSwordfight
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .deferred_elements_to_go()
                .iter()
                .any(|&(sequence, index)| {
                    engine
                        .orders
                        .sequence_manager
                        .get_element(sequence, index)
                        .unwrap()
                        .owner
                        == Some(owner)
                        && engine
                            .orders
                            .sequence_manager
                            .get_element(sequence, index)
                            .unwrap()
                            .command
                            == crate::element::Command::EnterSwordfight
                })
        );
    }

    fn prepare_approach(range: u16) -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
        let (mut engine, mut assets, owner, friend, target) = actors();
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "approach_test.scs",
        ));
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("approach weapon"));
        ai.hth_weapon_id = 1;
        ai.sword_range = 1;
        std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].distance
            [WeaponDistance::Default as usize] = range;
        engine.enter_ai_think_frame(owner);
        (engine, assets, owner, friend, target)
    }

    fn move_actor(engine: &mut EngineInner, actor: EntityId, x: f32, y: f32) {
        engine.place(actor, WorldPoint3D::new(x, y, 0.0));
    }

    fn pending_moves(
        engine: &EngineInner,
        owner: EntityId,
    ) -> Vec<(crate::sequence::SequenceId, usize)> {
        engine
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .iter()
            .copied()
            .filter(|&(id, index)| {
                engine
                    .orders
                    .sequence_manager
                    .get_element(id, index)
                    .is_some_and(|element| {
                        element.owner == Some(owner)
                            && element.state != crate::sequence::SequenceState::Interrupted
                            && matches!(
                                element.command,
                                crate::element::Command::Move
                                    | crate::element::Command::MoveWaiting
                            )
                    })
            })
            .collect()
    }

    fn install_stopping_state_callback(
        engine: &mut EngineInner,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::engine::test_support::asm::*;
        use crate::natives::NativeFn;
        engine.actor_mut(owner).script_class = "StopOnState".into();
        let quads = vec![
            q_begin_function(0, 4),
            q_native_call(NativeFn::ThisActor as u32),
            q_aff1_native_get_return(0xc000),
            q_aff0_iconstant(0xc004, 0),
            q_native_param(0xc000),
            q_native_param(0xc004),
            q_native_call(NativeFn::GetCustomNPCValue as u32),
            q_aff1_native_get_return(0xc008),
            q_aff0_iconstant(0xc00c, 1),
            q_iadd(0xc008, 0xc008, 0xc00c),
            q_native_param(0xc000),
            q_native_param(0xc004),
            q_native_param(0xc008),
            q_native_call(NativeFn::SetCustomNPCValue as u32),
            q_native_param(0xc000),
            q_native_call(NativeFn::StopActor as u32),
            q_return_val(0xc00c),
            q_end_function(),
        ];

        engine.scripts.mission = Some(
            crate::engine::test_support::extra_engine_combat::filter_ai_event_mission(
                "approach_stop.scs",
                "StopOnState",
                16,
                quads,
            ),
        );
        engine.scripts.mission.as_mut().unwrap().bind_actor(
            crate::natives::ScriptHandleCodec::actor_handle(owner),
            "StopOnState",
        );
        engine.attach_script_bindings(assets);
    }

    #[test]
    fn reconsider_approach_resolves_position_after_reciprocal_retarget() {
        let (mut engine, assets, owner, friend, target) = prepare_approach(50);
        let replacement = engine.add_test_entity(make_test_pc(Posture::Upright));
        let sector = engine.sector_of(target);
        engine.elem_mut(replacement).set_sector(sector);
        for (id, x) in [
            (owner, 100.0),
            (friend, 1100.0),
            (target, 1140.0),
            (replacement, 140.0),
        ] {
            move_actor(&mut engine, id, x, 500.0);
        }
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(friend, format_args!("exchange partner"));
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToEnemy;
        ai.base.primary_target = Some(AiEntityHandle::new(replacement.index()));
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        let ai = engine.enemy(owner);
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(replacement.index()))
        );
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(
            engine.enemy(friend).base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
        let pending = engine.orders.sequence_manager.deferred_elements_to_go();
        assert!(pending.iter().any(|&(id, index)| {
            engine
                .orders
                .sequence_manager
                .get_element(id, index)
                .is_some_and(|element| {
                    element.owner == Some(owner)
                        && element.state != crate::sequence::SequenceState::Interrupted
                        && element.command == crate::element::Command::EnterSwordfight
                })
        }));
    }

    #[test]
    fn reconsider_approach_registers_move_before_state_callback_and_preserves_same_state_move() {
        for same_state in [false, true] {
            let (mut engine, assets, owner, _, target) = prepare_approach(50);
            move_actor(&mut engine, owner, 1773.7925, 2523.631);
            move_actor(&mut engine, target, 1731.4956, 2379.8796);
            let ai = engine.enemy_mut(owner);
            ai.base.current_substate = if same_state {
                Substate::AttackingRunningToEnemy
            } else {
                Substate::AttackingTooProudToAttackApproach
            };
            install_stopping_state_callback(&mut engine, &assets, owner);
            engine.execute_ai_reconsider_enemy_approach(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner,
                true,
            );
            let ai = engine.enemy(owner);
            assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
            assert_eq!(ai.base.stop_before_end_of_path_distance, 50);
            let count = engine.npc(owner).custom_values[0];
            if same_state {
                assert_eq!(count, 0, "same substate must not manufacture a callback");
                assert_eq!(pending_moves(&engine, owner).len(), 1);
            } else {
                assert_eq!(count, 1);
                assert!(
                    pending_moves(&engine, owner).is_empty(),
                    "the callback must be able to stop the already registered approach"
                );
            }
            assert_eq!(engine.ai_think_depth(), 1);
        }
    }

    #[test]
    fn reconsider_approach_already_near_engages_without_charging_movement() {
        let (mut engine, assets, owner, _, target) = prepare_approach(150);
        move_actor(&mut engine, owner, 100.0, 100.0);
        move_actor(&mut engine, target, 200.0, 100.0);
        engine.enemy_mut(owner).sword_is_charge_weapon = true;
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert!(!ai.base.already_on_point);
        assert!(pending_moves(&engine, owner).is_empty());
        assert_eq!(engine.ai_think_depth(), 1);
    }

    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::engine::test_support::{
        actors::{make_test_ai_soldier, make_test_pc},
        square_sector,
    };

    fn actors() -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        let (sector, _) = crate::engine::test_support::extra_engine_combat::square_sector_map(
            &mut engine,
            (256, 256),
            (5000.0, 5000.0),
        );
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let friend = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let target = engine.add_test_entity(make_test_pc(Posture::Upright));
        for (id, x, y) in [
            (owner, 500.0, 500.0),
            (friend, 2000.0, 2000.0),
            (target, 500.0, 200.0),
        ] {
            let entity = engine.ent_mut(id);
            entity.element_data_mut().set_sector(Some(sector));
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, y, 0.0));
            entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
            entity
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-10.0, -5.0),
                    crate::coordinates::MapVec::new(10.0, 5.0),
                ));
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("approach fixture"));
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
        ai.list_them = vec![target.index()];
        (engine, assets, owner, friend, target)
    }

    #[test]
    fn rider_corridor_reads_friends_outside_the_old_nearby_window() {
        let (mut engine, _assets, owner, friend, target) = actors();
        let owner_position = Position {
            x: f32::from_bits(0x44c7_1d8e),
            y: f32::from_bits(0x4421_e39e),
            ..engine.live_ai_position(owner)
        };
        for (id, x, y) in [
            (owner, owner_position.x, owner_position.y),
            (
                target,
                f32::from_bits(0x4474_03e3),
                f32::from_bits(0x43bb_a89f),
            ),
            (
                friend,
                f32::from_bits(0x447b_182c),
                f32::from_bits(0x43b8_eb5e),
            ),
        ] {
            engine.place(id, WorldPoint3D::new(x, y, 0.0));
        }
        assert!(
            engine
                .live_rider_attack_destination(owner, target, owner_position, 11)
                .is_none()
        );
        engine.place(friend, WorldPoint3D::new(3000.0, 3000.0, 0.0));
        assert!(
            engine
                .live_rider_attack_destination(owner, target, owner_position, 11)
                .is_some()
        );
    }

    #[test]
    fn rider_charge_finishes_focus_and_retains_live_target_position() {
        for (distance, passing) in [(200.0, false), (50.0, true)] {
            let (mut engine, assets, owner, _, target) = actors();
            let Entity::Soldier(soldier) = engine.ent_mut(owner) else {
                unreachable!()
            };
            soldier.soldier.rider = true;
            engine
                .ent_mut(owner)
                .position_iface_mut()
                .set_direction(crate::position_interface::Direction::from_raw(0));
            engine.place(target, WorldPoint3D::new(500.0, 500.0 - distance, 0.0));
            let position = engine.live_ai_position(target);
            assert!(engine.execute_ai_maybe_make_rider_attack(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner
            ));
            let ai = engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("rider result"));
            assert_eq!(ai.seek_position, position);
            assert_eq!(ai.primary_target, Some(AiEntityHandle::new(target.index())));
            assert_eq!(
                ai.current_substate,
                if passing {
                    Substate::AttackingRiderChargingPassing
                } else {
                    Substate::AttackingRiderChargingApproaching
                }
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .expect_ai_actor_data(owner, format_args!("rider focus"))
                    .follow_target,
                Some(target)
            );
            assert_eq!(
                engine
                    .world
                    .entities
                    .expect_ai_actor_data(owner, format_args!("rider eyes"))
                    .eye_status,
                if passing {
                    crate::element::EyeStatus::LookForward
                } else {
                    crate::element::EyeStatus::Follow
                }
            );
        }
    }

    #[test]
    fn rider_geometry_uses_raw_position_even_when_ai_position_is_substituted() {
        let (mut engine, _, owner, carrier, target) = actors();
        engine.place(carrier, WorldPoint3D::new(500.0, 450.0, 0.0));
        let target_entity = engine.ent_mut(target);
        target_entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(500.0, 550.0, 0.0));
        target_entity
            .element_data_mut()
            .set_posture(Posture::OnShoulders);
        target_entity.human_data_mut().unwrap().carrier = Some(carrier);
        assert_eq!(engine.live_ai_position(target).y, 450.0);
        assert!(
            engine
                .live_rider_attack_destination(owner, target, engine.live_ai_position(owner), 0)
                .is_none()
        );
    }

    #[test]
    fn rider_charge_uses_a_live_persistent_target_without_a_nearby_projection() {
        let (mut engine, assets, owner, target, _) = actors();
        let Entity::Soldier(soldier) = engine.ent_mut(owner) else {
            unreachable!()
        };
        soldier.soldier.rider = true;
        let Entity::Soldier(soldier) = engine.ent_mut(target) else {
            unreachable!()
        };
        soldier.soldier.cached_camp = Camp::Royalists;
        engine
            .ent_mut(owner)
            .position_iface_mut()
            .set_direction(crate::position_interface::Direction::from_raw(0));
        engine.place(target, WorldPoint3D::new(500.0, 450.0, 0.0));
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("rider target"))
            .primary_target = Some(AiEntityHandle::new(target.index()));
        assert!(engine.execute_ai_maybe_make_rider_attack(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner
        ));
        assert_eq!(
            engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("rider result"))
                .current_substate,
            Substate::AttackingRiderChargingPassing
        );
    }

    #[test]
    fn approach_resolves_current_carrier_without_a_target_snapshot() {
        let (mut engine, assets, owner, carrier, target) = actors();
        let Entity::Soldier(soldier) = engine.ent_mut(carrier) else {
            unreachable!()
        };
        soldier.soldier.cached_camp = Camp::Royalists;
        let entity = engine.ent_mut(target);
        entity.element_data_mut().set_posture(Posture::OnShoulders);
        entity.human_data_mut().unwrap().carrier = Some(carrier);
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        assert_eq!(
            engine
                .world
                .entities
                .expect_ai_controller(owner, format_args!("carrier result"))
                .primary_target,
            Some(AiEntityHandle::new(carrier.index()))
        );
    }

    #[test]
    fn already_swordfighting_approach_keeps_the_short_timer_without_target_data() {
        let (mut engine, assets, owner, _, target) = actors();
        engine.human_mut(owner).opponents.push(target);
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("combat fixture"));
        ai.primary_target = None;
        ai.current_substate = Substate::AttackingSwordfight;
        engine.execute_ai_reconsider_enemy_approach(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            false,
        );
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("combat result"));
        assert_eq!(ai.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.when_does_timer_ring, engine.control.frame_counter + 30);
    }
}
