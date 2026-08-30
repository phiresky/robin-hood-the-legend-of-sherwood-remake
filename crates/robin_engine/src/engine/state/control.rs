use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

use crate::engine::{SimConfig, SimulationGateState, SimulationRng};

/// Deterministic clock, random stream, and global simulation-rate controls.
///
/// This owns state only; [`crate::engine::EngineInner`] remains responsible for
/// phase ordering and lifecycle orchestration.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct SimulationControl {
    pub(crate) frame_counter: u32,
    pub(crate) simulation_gates: SimulationGateState,
    pub(crate) speed: f32,
    pub(crate) speed_int: u16,
    pub(crate) chorus_timer: u16,
    pub(crate) rng: SimulationRng,
    pub(crate) sim_config: SimConfig,
    /// Exact construction checkpoint used when a loaded save later requests
    /// a full mission restart. Unlike `rng`, this never advances.
    pub(crate) mission_start_rng_seed: u64,
    pub(crate) mission_start_sim_config: SimConfig,
    pub(crate) fast_forward: bool,
    /// A completed `PerformHourglass` is followed by the presentation-only
    /// `RHElementArrow::Refresh` pass. Parity snapshots sit between those
    /// calls, so Rust applies this pending pass immediately before the next
    /// hourglass instead of mutating arrow sprites during their entity tick.
    #[serde(default)]
    pub(crate) arrow_refresh_pending: bool,
    /// Universal frame of the most recently displayed popup scroll.
    ///
    /// Original's `RHMenuPopupScroll::ulLastFrame` suppresses the
    /// colorized-background constructor (and therefore its nested Refresh)
    /// for a second popup displayed in the same engine frame.
    #[serde(default)]
    pub(crate) popup_scroll_last_display_frame: Option<u32>,
    /// Captured Original results for the stale-sprite `0xffff` action-point
    /// over-read. The C++ getter indexes beyond `auwDelay`, so this value is
    /// allocator residue rather than reproducible simulation state. Parity
    /// replays may supply the observed wrapped `SWORD`, keyed by the
    /// proposer's and target's Original creation orders, for the current
    /// frame only.
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub(crate) original_impossible_action_done_deadlines: BTreeMap<(u32, u32), VecDeque<i16>>,
}

impl SimulationControl {
    pub(crate) fn new(seed: u64, sim_config: SimConfig) -> Self {
        sim_config
            .validate()
            .expect("cannot start simulation with invalid difficulty rules");
        Self {
            frame_counter: 0,
            simulation_gates: SimulationGateState::default(),
            speed: 1.0,
            speed_int: 0,
            chorus_timer: 0,
            rng: SimulationRng::with_seed(seed),
            sim_config,
            mission_start_rng_seed: seed,
            mission_start_sim_config: sim_config,
            fast_forward: false,
            arrow_refresh_pending: false,
            popup_scroll_last_display_frame: None,
            original_impossible_action_done_deadlines: BTreeMap::new(),
        }
    }

    pub(crate) fn simulation_context(&self) -> crate::sim_rng::SimulationContext {
        self.rng.context(self.sim_config)
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

    /// Record one completed popup-scroll display and report whether its menu
    /// background takes Original's nested-Refresh path.
    pub(crate) fn begin_popup_scroll_display(&mut self) -> bool {
        let refresh = self.popup_scroll_last_display_frame != Some(self.frame_counter);
        self.popup_scroll_last_display_frame = Some(self.frame_counter);
        refresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_control_has_the_canonical_running_state() {
        let control = SimulationControl::new(17, SimConfig::default());

        assert_eq!(control.frame_counter, 0);
        assert!(!control.engine_locked());
        assert!(!control.actors_frozen());
        assert_eq!(control.speed, 1.0);
        assert_eq!(control.speed_int, 0);
        assert_eq!(control.chorus_timer, 0);
        assert_eq!(control.rng.seed(), 17);
        assert!(!control.fast_forward);
        assert!(!control.arrow_refresh_pending);
        assert_eq!(control.popup_scroll_last_display_frame, None);
    }

    #[test]
    fn popup_scroll_refreshes_only_once_per_universal_frame() {
        let mut control = SimulationControl::new(17, SimConfig::default());

        assert!(control.begin_popup_scroll_display());
        assert!(!control.begin_popup_scroll_display());

        control.frame_counter += 1;
        assert!(control.begin_popup_scroll_display());
    }
}
