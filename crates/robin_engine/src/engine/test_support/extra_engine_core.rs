//! `t_*_with` adapters: the `t_*` entry-point shortcuts for tests that thread
//! one [`SimulationContext`] through several engine calls, so its RNG stream
//! keeps advancing across them instead of restarting from the seed per call.

use crate::engine::{EngineInner, HostDisplayState, LevelAssets};
use crate::sequence::{Sequence, SequenceElement, SequenceId};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(crate) fn t_launch_element_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        elem: SequenceElement,
    ) -> SequenceId {
        self.launch_element(sim, assets, elem)
    }

    pub(crate) fn t_launch_sequence_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq: Sequence,
    ) -> SequenceId {
        self.launch_sequence(sim, assets, seq)
    }

    pub(crate) fn t_element_in_progress_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_in_progress(sim, assets, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_element_terminated_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_postpone_element_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.postpone_element(sim, assets, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_tick_actor_owner_envelopes_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_owner_envelopes(sim, assets);
    }

    pub(crate) fn t_hourglass_phase_sequences_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
    ) {
        self.hourglass_phase_sequences(sim, &mut HostDisplayState::default(), assets);
    }
}
