//! One-line test shortcuts: entity queries that panic on missing data, and
//! thin adapters over wide-signature entry points that supply the standard
//! test simulation context and a throwaway script-call list.
//!
//! See `docs/TEST_HELPERS.md` for the verbose pattern each one replaces.
// Shared vocabulary: not every helper has a caller in every build.
#![allow(dead_code)]

use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{
    ActionState, ActorData, ElementData, Entity, EntityId, HumanData, NpcData, PcData, Posture,
};
use crate::engine::TickCtx;
use crate::engine::{EngineInner, HostDisplayState, LevelAssets};
use crate::position_interface::SectorHandle;
use crate::sequence::{CascadeFlags, Sequence, SequenceElement, SequenceId};

fn sim() -> crate::sim_rng::SimulationContext {
    crate::sim_rng::test_context()
}

/// Entity queries. Every accessor panics, naming the entity and the missing
/// component, instead of returning a fabricated value.
impl EngineInner {
    #[track_caller]
    pub(crate) fn ent<I: Into<EntityId>>(&self, id: I) -> &Entity {
        let id = id.into();
        self.get_entity(id)
            .unwrap_or_else(|| panic!("test entity {id:?} does not exist"))
    }

    #[track_caller]
    pub(crate) fn ent_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut Entity {
        let id = id.into();
        self.get_entity_mut(id)
            .unwrap_or_else(|| panic!("test entity {id:?} does not exist"))
    }

    #[track_caller]
    pub(crate) fn elem<I: Into<EntityId>>(&self, id: I) -> &ElementData {
        self.ent(id).element_data()
    }

    #[track_caller]
    pub(crate) fn elem_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut ElementData {
        self.ent_mut(id).element_data_mut()
    }

    #[track_caller]
    pub(crate) fn actor<I: Into<EntityId>>(&self, id: I) -> &ActorData {
        let id = id.into();
        self.ent(id)
            .actor_data()
            .unwrap_or_else(|| panic!("test entity {id:?} has no actor data"))
    }

    #[track_caller]
    pub(crate) fn actor_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut ActorData {
        let id = id.into();
        self.ent_mut(id)
            .actor_data_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no actor data"))
    }

    #[track_caller]
    pub(crate) fn human<I: Into<EntityId>>(&self, id: I) -> &HumanData {
        let id = id.into();
        self.ent(id)
            .human_data()
            .unwrap_or_else(|| panic!("test entity {id:?} has no human data"))
    }

    #[track_caller]
    pub(crate) fn human_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut HumanData {
        let id = id.into();
        self.ent_mut(id)
            .human_data_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no human data"))
    }

    #[track_caller]
    pub(crate) fn pc<I: Into<EntityId>>(&self, id: I) -> &PcData {
        let id = id.into();
        self.ent(id)
            .pc_data()
            .unwrap_or_else(|| panic!("test entity {id:?} has no PC data"))
    }

    #[track_caller]
    pub(crate) fn pc_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut PcData {
        let id = id.into();
        self.ent_mut(id)
            .pc_data_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no PC data"))
    }

    #[track_caller]
    pub(crate) fn npc<I: Into<EntityId>>(&self, id: I) -> &NpcData {
        let id = id.into();
        self.ent(id)
            .npc_data()
            .unwrap_or_else(|| panic!("test entity {id:?} has no NPC data"))
    }

    #[track_caller]
    pub(crate) fn npc_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut NpcData {
        let id = id.into();
        self.ent_mut(id)
            .npc_data_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no NPC data"))
    }

    #[track_caller]
    pub(crate) fn enemy<I: Into<EntityId>>(&self, id: I) -> &crate::ai_enemy::EnemyAi {
        let id = id.into();
        self.ent(id)
            .enemy_ai()
            .unwrap_or_else(|| panic!("test entity {id:?} has no enemy AI"))
    }

    #[track_caller]
    pub(crate) fn enemy_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut crate::ai_enemy::EnemyAi {
        let id = id.into();
        self.ent_mut(id)
            .enemy_ai_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no enemy AI"))
    }

    #[track_caller]
    pub(crate) fn ai_ctrl<I: Into<EntityId>>(&self, id: I) -> &crate::ai::AiController {
        let id = id.into();
        self.ent(id)
            .ai_controller()
            .unwrap_or_else(|| panic!("test entity {id:?} has no AI controller"))
    }

    #[track_caller]
    pub(crate) fn ai_ctrl_mut<I: Into<EntityId>>(&mut self, id: I) -> &mut crate::ai::AiController {
        let id = id.into();
        self.ent_mut(id)
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("test entity {id:?} has no AI controller"))
    }

    #[track_caller]
    pub(crate) fn motion_state_of<I: Into<EntityId>>(&self, id: I) -> crate::sprite::MotionState {
        self.actor(id).continuation.motion_state
    }

    #[track_caller]
    pub(crate) fn action_state_of<I: Into<EntityId>>(&self, id: I) -> ActionState {
        self.actor(id).action_state
    }

    #[track_caller]
    pub(crate) fn set_action_state_of<I: Into<EntityId>>(&mut self, id: I, state: ActionState) {
        self.actor_mut(id).action_state = state;
    }

    #[track_caller]
    pub(crate) fn pos_of<I: Into<EntityId>>(&self, id: I) -> WorldPoint3D {
        self.elem(id).position()
    }

    #[track_caller]
    pub(crate) fn map_pos_of<I: Into<EntityId>>(&self, id: I) -> MapPoint {
        self.elem(id).position_map()
    }

    #[track_caller]
    pub(crate) fn direction_of<I: Into<EntityId>>(&self, id: I) -> i16 {
        self.elem(id).direction()
    }

    #[track_caller]
    pub(crate) fn posture_of<I: Into<EntityId>>(&self, id: I) -> Posture {
        self.ent(id).posture()
    }

    #[track_caller]
    pub(crate) fn sector_of<I: Into<EntityId>>(&self, id: I) -> Option<SectorHandle> {
        self.elem(id).sector()
    }

    #[track_caller]
    pub(crate) fn place<I: Into<EntityId>>(&mut self, id: I, pos: WorldPoint3D) {
        self.elem_mut(id).set_position(pos);
    }

    #[track_caller]
    pub(crate) fn place_map<I: Into<EntityId>>(&mut self, id: I, pos: MapPoint) {
        self.elem_mut(id).set_position_map(pos);
    }

    #[track_caller]
    pub(crate) fn face<I: Into<EntityId>>(&mut self, id: I, direction: i16) {
        self.elem_mut(id).set_direction_instantly(direction);
    }

    #[track_caller]
    pub(crate) fn set_active<I: Into<EntityId>>(&mut self, id: I, active: bool) {
        self.elem_mut(id).active = active;
    }
}

/// Adapters that fix `sim` to [`crate::sim_rng::test_context`] and discard
/// the active-script list, for tests that observe neither.
impl EngineInner {
    pub(crate) fn t_launch_element(
        &mut self,
        assets: &LevelAssets,
        elem: SequenceElement,
    ) -> SequenceId {
        self.launch_element(TickCtx::new(&sim(), assets), elem)
    }

    pub(crate) fn t_launch_sequence(&mut self, assets: &LevelAssets, seq: Sequence) -> SequenceId {
        self.launch_sequence(TickCtx::new(&sim(), assets), seq)
    }

    pub(crate) fn t_element_in_progress(
        &mut self,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_in_progress(
            TickCtx::new(&sim(), assets),
            &mut Vec::new(),
            seq_id,
            elem_idx,
        );
    }

    pub(crate) fn t_element_terminated(
        &mut self,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_terminated(
            TickCtx::new(&sim(), assets),
            &mut Vec::new(),
            seq_id,
            elem_idx,
        );
    }

    pub(crate) fn t_element_interrupted(
        &mut self,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        self.element_interrupted(
            TickCtx::new(&sim(), assets),
            &mut Vec::new(),
            seq_id,
            elem_idx,
            flags,
        );
    }

    pub(crate) fn t_postpone_element(
        &mut self,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.postpone_element(
            TickCtx::new(&sim(), assets),
            &mut Vec::new(),
            seq_id,
            elem_idx,
        );
    }

    pub(crate) fn t_instruct_owner(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> bool {
        self.instruct_owner(
            TickCtx::new(&sim(), assets),
            &mut Vec::new(),
            owner,
            seq_id,
            elem_idx,
        )
    }

    pub(crate) fn t_tick_actor_owner_envelopes(&mut self, assets: &LevelAssets) {
        self.tick_actor_owner_envelopes(TickCtx::new(&sim(), assets));
    }

    pub(crate) fn t_hourglass_phase_sequences(&mut self, assets: &LevelAssets) {
        self.hourglass_phase_sequences(
            TickCtx::new(&sim(), assets),
            &mut HostDisplayState::default(),
        );
    }

    /// Launch a single element and immediately mark it in progress; returns
    /// its sequence.
    pub(crate) fn t_launch_in_progress(
        &mut self,
        assets: &LevelAssets,
        elem: SequenceElement,
    ) -> SequenceId {
        let seq_id = self.t_launch_element(assets, elem);
        self.t_element_in_progress(assets, seq_id, 0);
        seq_id
    }
}
