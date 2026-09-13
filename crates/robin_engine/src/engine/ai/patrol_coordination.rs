use super::*;

impl EngineInner {
    /// Apply facing from the two actor values it actually reads. In particular,
    /// this runs after coordinate Think, so callback changes are visible.
    fn instruct_patrol_direction(&mut self, member: EntityId, direction: u16) {
        let entity = self
            .world
            .entities
            .expect_entity(member, format_args!("patrol direction member"));
        let current_direction = entity.element_data().direction() as u16;
        let action_state = entity
            .actor_data()
            .expect("patrol member has no actor data")
            .action_state;
        self.world
            .entities
            .expect_ai_controller_mut(member, format_args!("patrol direction member"))
            .set_instructed_patrol_direction(direction, current_direction, action_state);
    }

    pub(in crate::engine) fn drain_patrol_direction_broadcast_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) {
        let Some(ai) = self
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
        else {
            return;
        };
        let Some(direction) = ai.outbox.patrol.direction_broadcast.take() else {
            return;
        };
        let member_count = ai.patrol.len();
        for index in 0..member_count {
            let member = *self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("patrol direction chief"))
                .patrol
                .get(index)
                .expect("patrol shrank during direction callback");
            self.instruct_patrol_direction(member, direction);
            // Register turns now; owner instruction belongs to the later
            // sequence-manager pass, as with coordinate Think below.
            self.drain_direct_ai_owner_boundary(sim, member, assets);
        }
    }

    /// Run one chief's patrol refresh. Only authored formation destinations and
    /// dispatch arguments cross callbacks; actors and obstacles are read from
    /// their owners, without an all-NPC patrol snapshot.
    pub(in crate::engine) fn tick_patrol_coordination_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::ai::{AiState, Stimulus, StimulusType, Substate};

        if self.actors_frozen() || self.is_very_very_busy(owner) {
            return;
        }
        let Some(ai) = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
        else {
            return;
        };
        if !ai.needs_patrol_reinit && ai.patrol.is_empty() && ai.missed_patrol_members.is_empty() {
            return;
        }
        if ai.needs_patrol_reinit {
            let theoretical = ai.theoretical_patrol.clone();
            self.assemble_patrol_for_npc(assets, owner, &theoretical);
        }

        let ai = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("patrol chief"));
        if (ai.patrol.is_empty() && ai.missed_patrol_members.is_empty())
            || ai.patrol_stopped
            || ai.current_state != AiState::Default
            || ai.current_substate == Substate::DefaultPatrolChiefReturnToPatrol
            || ai.patrol_path.is_none()
        {
            return;
        }

        let frame = self.control.frame_counter;
        let position = self.live_ai_position(owner);
        let entity = self.expect_entity(owner, "patrol chief");
        let direction = entity.element_data().direction() as u8;
        let bounds = *entity.position_iface().get_move_box();
        let bounds = if bounds.is_somewhere() {
            crate::coordinates::MoveBox::from_coords(
                bounds.x_min() - 3.0,
                bounds.y_min() - 3.0,
                bounds.x_max() + 3.0,
                bounds.y_max() + 3.0,
            )
        } else {
            crate::coordinates::MoveBox::new()
        };
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("patrol chief"));
        let path = ai
            .patrol_path
            .as_mut()
            .expect("validated patrol path disappeared");
        path.add_history_entry(position, direction);
        if frame & 7 != 0 {
            return;
        }

        // Formation geometry and loop extent are fixed before callbacks.
        // Membership and distance are read at each indexed call site.
        let positions =
            path.compute_patrol_positions(ai.patrol.len(), Some(&self.world.fast_grid), &bounds);
        for (index, (target, direction)) in positions.into_iter().enumerate() {
            let member = *self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("patrol chief"))
                .patrol
                .get(index)
                .expect("patrol shrank during coordinate callback");
            let current = self.live_ai_position(member);
            if !((current.x - target.x)
                .abs()
                .max((current.y - target.y).abs())
                > 3.0)
            {
                continue;
            }
            let entity = self.expect_entity(member, "patrol coordinate member");
            let scratch = self.build_sim_scratch(assets);
            let ctx = self.ai_context_from_entity(entity, frame, None, &scratch, assets);
            let tick = self.build_npc_tick_data(sim, member, assets);
            let stimulus = Stimulus::with_position(StimulusType::CallPatrolCoordinate, target);
            self.debug_patrol_turn_lifecycle("before_coordinate_think", member);
            self.dispatch_think_with_drain(sim, member, &stimulus, &ctx, &tick, assets);
            // Construct Move before applying direction, but leave its deferred
            // InstructOwner for the normal sequence-manager phase.
            self.drain_pending_move_requests_for_owner(sim, member);
            self.debug_patrol_turn_lifecycle("after_coordinate_think", member);
            let member = *self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("patrol chief after coordinate"))
                .patrol
                .get(index)
                .expect("patrol shrank during coordinate callback");
            self.instruct_patrol_direction(member, direction);
            self.debug_patrol_turn_lifecycle("after_instructed_direction_emit", member);
            self.drain_direct_ai_owner_boundary(sim, member, assets);
            self.debug_patrol_turn_lifecycle("after_instructed_direction_drain", member);
        }
        self.reacquire_patrol_members(assets, owner);
    }

    fn reacquire_patrol_members(&mut self, assets: &LevelAssets, owner: EntityId) {
        let missed = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("patrol chief"))
            .missed_patrol_members
            .clone();
        let mut reacquired = Vec::new();
        for (index, member) in missed.into_iter().enumerate() {
            let entity = self.expect_entity(member, "missed patrol member");
            let npc = entity
                .ai_actor_data()
                .expect("missed patrol member has no AI actor data");
            let able_to_help = match entity {
                Entity::Soldier(soldier) => crate::ai_enemy::soldier_is_able_to_help_state(
                    !entity.is_dead() && !soldier.human.unconscious,
                    npc.ai_state(),
                    npc.ai_substate(),
                ),
                _ => false,
            };
            if missed_patrol_member_reacquired(
                true,
                || self.patrol_member_visible(assets, owner, member),
                able_to_help,
                npc.ai_state(),
            ) {
                self.world
                    .entities
                    .expect_ai_controller_mut(member, format_args!("reacquired patrol member"))
                    .patrol_chief = Some(owner);
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("patrol chief"))
                    .patrol
                    .push(member);
                reacquired.push(index);
            }
        }
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("patrol chief"));
        for index in reacquired.into_iter().rev() {
            ai.missed_patrol_members.remove(index);
        }
    }
}
