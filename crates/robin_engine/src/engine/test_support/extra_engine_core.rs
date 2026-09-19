//! `t_*_with` adapters: the `t_*` entry-point shortcuts for tests that thread
//! one [`SimulationContext`] through several engine calls, so its RNG stream
//! keeps advancing across them instead of restarting from the seed per call.

use crate::engine::{EngineInner, HostDisplayState};
use crate::engine::{LevelAssets, TickCtx};
use crate::sequence::SequenceElementRef;
use crate::sequence::{Sequence, SequenceElement, SequenceId};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(crate) fn t_launch_element_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        elem: SequenceElement,
    ) -> SequenceId {
        self.launch_element(TickCtx::new(sim, assets), elem)
    }

    pub(crate) fn t_launch_sequence_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq: Sequence,
    ) -> SequenceId {
        self.launch_sequence(TickCtx::new(sim, assets), seq)
    }

    pub(crate) fn t_element_in_progress_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_in_progress(
            TickCtx::new(sim, assets),
            &mut Vec::new(),
            SequenceElementRef::new(seq_id, elem_idx),
        );
    }

    pub(crate) fn t_element_terminated_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_terminated(
            TickCtx::new(sim, assets),
            &mut Vec::new(),
            SequenceElementRef::new(seq_id, elem_idx),
        );
    }

    pub(crate) fn t_postpone_element_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.postpone_element(
            TickCtx::new(sim, assets),
            &mut Vec::new(),
            SequenceElementRef::new(seq_id, elem_idx),
        );
    }

    pub(crate) fn t_tick_actor_owner_envelopes_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_owner_envelopes(TickCtx::new(sim, assets));
    }

    pub(crate) fn t_hourglass_phase_sequences_with(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
    ) {
        self.hourglass_phase_sequences(TickCtx::new(sim, assets), &mut HostDisplayState::default());
    }
}
