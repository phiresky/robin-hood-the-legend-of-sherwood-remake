//! `t_*_with` adapters: the `t_*` entry-point shortcuts for tests that thread
//! one [`SimulationContext`] through several engine calls, so its RNG stream
//! keeps advancing across them instead of restarting from the seed per call.

use crate::engine::TickCtx;
use crate::engine::{EngineInner, HostDisplayState};
use crate::sequence::{Sequence, SequenceElement, SequenceId};

impl EngineInner {
    pub(crate) fn t_launch_element_with(
        &mut self,
        tcx: TickCtx<'_>,
        elem: SequenceElement,
    ) -> SequenceId {
        self.launch_element(tcx, elem)
    }

    pub(crate) fn t_launch_sequence_with(&mut self, tcx: TickCtx<'_>, seq: Sequence) -> SequenceId {
        self.launch_sequence(tcx, seq)
    }

    pub(crate) fn t_element_in_progress_with(
        &mut self,
        tcx: TickCtx<'_>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_in_progress(tcx, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_element_terminated_with(
        &mut self,
        tcx: TickCtx<'_>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.element_terminated(tcx, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_postpone_element_with(
        &mut self,
        tcx: TickCtx<'_>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        self.postpone_element(tcx, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(crate) fn t_tick_actor_owner_envelopes_with(&mut self, tcx: TickCtx<'_>) {
        self.tick_actor_owner_envelopes(tcx);
    }

    pub(crate) fn t_hourglass_phase_sequences_with(&mut self, tcx: TickCtx<'_>) {
        self.hourglass_phase_sequences(tcx, &mut HostDisplayState::default());
    }
}
