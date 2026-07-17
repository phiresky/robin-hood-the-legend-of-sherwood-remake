use serde::{Deserialize, Serialize};

use crate::engine::{SimulationGateState, SimulationRng};

/// Deterministic clock, random stream, and global simulation-rate controls.
///
/// This owns state only; [`crate::engine::EngineInner`] remains responsible for
/// phase ordering and lifecycle orchestration.
#[derive(Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct SimulationControl {
    pub(crate) frame_counter: u32,
    pub(crate) simulation_gates: SimulationGateState,
    pub(crate) speed: f32,
    pub(crate) speed_int: u16,
    pub(crate) chorus_timer: u16,
    pub(crate) rng: SimulationRng,
    pub(crate) fast_forward: bool,
}

impl SimulationControl {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            frame_counter: 0,
            simulation_gates: SimulationGateState::default(),
            speed: 1.0,
            speed_int: 0,
            chorus_timer: 0,
            rng: SimulationRng::with_seed(seed),
            fast_forward: false,
        }
    }

    pub(crate) fn enter_rng_scope(&mut self) {
        self.rng.enter_scope();
    }

    pub(crate) fn leave_rng_scope(&mut self) {
        self.rng.leave_scope();
    }

    pub(crate) fn engine_locked(&self) -> bool {
        self.simulation_gates.engine_locked()
    }

    pub(crate) fn set_engine_locked(&mut self, locked: bool) {
        self.simulation_gates.set_engine_locked(locked);
    }

    pub(crate) fn actors_frozen(&self) -> bool {
        self.simulation_gates.actors_frozen()
    }

    pub(crate) fn set_actors_frozen(&mut self, frozen: bool) {
        self.simulation_gates.set_actors_frozen(frozen);
    }

    #[cfg(test)]
    pub(crate) fn fade_freeze_frames_remaining(&self) -> u32 {
        self.simulation_gates.fade_freeze_frames_remaining()
    }

    pub(crate) fn set_fade_freeze_frames_remaining(&mut self, frames: u32) {
        self.simulation_gates
            .set_fade_freeze_frames_remaining(frames);
    }

    pub(crate) fn consume_fade_freeze_frame(&mut self) -> bool {
        self.simulation_gates.consume_fade_freeze_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_control_has_the_canonical_running_state() {
        let control = SimulationControl::new(17);

        assert_eq!(control.frame_counter, 0);
        assert!(!control.engine_locked());
        assert!(!control.actors_frozen());
        assert_eq!(control.speed, 1.0);
        assert_eq!(control.speed_int, 0);
        assert_eq!(control.chorus_timer, 0);
        assert_eq!(control.rng.seed(), 17);
        assert!(!control.fast_forward);
    }
}
