//! Actor lifecycle diagnostics and required human-handle resolution.
use super::*;

impl EngineInner {
    #[inline(never)]
    pub(in crate::engine) fn debug_building_exit_wait_event_view(
        &self,
        owner: EntityId,
        queue_index: usize,
        stimulus: &crate::ai::Stimulus,
    ) {
        if !building_exit_wait_owner_debug_enabled()
            || stimulus.stimulus_type != crate::ai::StimulusType::EventView
        {
            return;
        }
        let (target_handle, target_creation_order) = match stimulus.info {
            crate::ai::StimulusInfo::Human(handle) => (
                Some(handle),
                self.entity_id_for_index(handle.get())
                    .map(|target| self.world.original_creation_order(target)),
            ),
            _ => (None, None),
        };
        eprintln!(
            "BEXITWAIT {{\"event\":\"queued_event_view\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"queue_index\":{queue_index},\"target_handle\":{target_handle:?},\"target_creation_order\":{target_creation_order:?}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
        );
    }

    #[inline(never)]
    pub(in crate::engine) fn debug_building_exit_wait_pc_route(
        &self,
        owner: EntityId,
        source_sector: crate::position_interface::SectorHandle,
        goal_sector: crate::position_interface::SectorHandle,
    ) {
        if !building_exit_wait_owner_debug_enabled() {
            return;
        }
        eprintln!(
            "BEXITWAIT {{\"event\":\"pc_door_fight_route\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"source_sector\":{},\"goal_sector\":{}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
            source_sector.get(),
            goal_sector.get(),
        );
    }

    #[inline(never)]
    pub(in crate::engine) fn debug_refresh_view_lifecycle(
        &self,
        stage: &str,
        npc_id: EntityId,
        derived_tail_order_type: Option<crate::order::OrderType>,
    ) {
        let gate = refresh_view_lifecycle_debug_gate();
        if !gate.enabled()
            || self.control.frame_counter < gate.filter(0).unwrap_or(0)
            || self.control.frame_counter > gate.filter(1).unwrap_or(u32::MAX)
        {
            return;
        }
        let creation_order = self.world.original_creation_order(npc_id);
        if !gate.matches([None, None, Some(creation_order)]) {
            return;
        }
        let entity = self
            .world
            .entities
            .expect_entity(npc_id, format_args!("RVLIFE owner at stage {stage}"));
        let Some(npc) = entity.ai_actor_data() else {
            return;
        };
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not an actor at stage {stage}",
                npc_id.index()
            )
        });
        let human = entity.human_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not human at stage {stage}",
                npc_id.index()
            )
        });
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let installed_order = actor
            .installed_order
            .map(|handle| handle.resolve(&self.orders.sequence_manager).order_type as u32)
            .map_or(-1_i64, i64::from);
        let derived_tail_order = derived_tail_order_type
            .map(|order| order as u32)
            .map_or(-1_i64, i64::from);
        let direction = entity.element_data().direction();
        let view_direction = npc.view_direction;
        let left_side = npc.view_left_side;
        let right_side = npc.view_right_side;
        let half_aperture = npc.real_half_aperture;
        let angle = npc.view_angle;
        let angle_step = npc.view_angle_step;
        eprintln!(
            "RVLIFE {{\"engine\":\"rust\",\"seq\":{sequence},\"stage\":{stage:?},\"frame\":{},\"owner_slot\":{},\"creation_order\":{creation_order},\"eye_status\":{},\"alpha_start\":{},\"radius_goal\":{},\"radius_step\":{},\"radius\":{},\"active\":{},\"unconscious\":{},\"tied\":{},\"dead\":{},\"frozen_all\":{},\"installed_order\":{installed_order},\"derived_tail_order\":{derived_tail_order},\"motion_state\":{},\"execution_frozen\":{},\"direction\":{direction},\"direction_old\":{},\"view_transition\":{},\"angle_bits\":{},\"angle_step_bits\":{},\"real_half_aperture_bits\":{},\"view_direction_bits\":[{},{}],\"left_side_bits\":[{},{}],\"right_side_bits\":[{},{}]}}",
            self.control.frame_counter,
            npc_id.index(),
            npc.eye_status as u8,
            npc.view_alpha_start,
            npc.view_radius_goal,
            npc.view_radius_step,
            npc.view_radius,
            entity.element_data().active,
            human.unconscious,
            entity.element_data().posture() == crate::element::Posture::Tied,
            entity.is_dead(),
            self.actors_frozen(),
            actor.continuation.motion_state as u8,
            actor.execution_frozen,
            npc.direction_old,
            npc.view_transition,
            angle.to_bits(),
            angle_step.to_bits(),
            half_aperture.to_bits(),
            view_direction[0].to_bits(),
            view_direction[1].to_bits(),
            left_side[0].to_bits(),
            left_side[1].to_bits(),
            right_side[0].to_bits(),
            right_side[1].to_bits(),
        );
    }

    /// Resolve an AI `HumanHandle` back through the original sparse element
    /// table without inventing an entity kind.  AI still stores these handles
    /// as raw slots, so a target can be a PC, soldier, or civilian.
    pub(in crate::engine) fn expect_human_id_for_ai_handle(
        &self,
        handle: crate::ai::HumanHandle,
        context: &str,
    ) -> EntityId {
        let id = self.expect_entity_id_for_index(handle, context);
        assert!(
            self.world
                .entities
                .get(id)
                .is_some_and(crate::element::Entity::is_human),
            "{context}: entity in raw slot {handle} is not human"
        );
        id
    }
}
