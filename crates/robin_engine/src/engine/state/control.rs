use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

use crate::engine::{RankedSimulationPolicy, SimConfig, SimulationGateState, SimulationRng};

/// Deterministic clock, random stream, and global simulation-rate controls.
///
/// This owns state only; [`crate::engine::EngineInner`] remains responsible for
/// phase ordering and lifecycle orchestration.
#[derive(Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)]
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
    /// Process-local ranked command/config admission capability. Its public
    /// identity is bound by the rules-config digest and is never reconstructed
    /// from an ordinary save or multiplayer snapshot.
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub(crate) ranked_simulation_policy: Option<RankedSimulationPolicy>,
    /// A completed simulation tick is followed by presentation-only entity
    /// `Refresh` work. Parity snapshots sit between those calls, so Rust
    /// applies the pending arrow and frame-sound work immediately before the
    /// next hourglass instead of mutating sprite state during the entity tick.
    ///
    /// The field keeps its original name for native snapshot compatibility.
    pub(crate) arrow_refresh_pending: bool,
    /// Universal frame of the most recently displayed popup scroll.
    ///
    /// The original game's popup-scroll frame tracking suppresses the
    /// colorized-background constructor (and therefore its nested Refresh)
    /// for a second popup displayed in the same engine frame.
    pub(crate) popup_scroll_last_display_frame: Option<u32>,
    /// Captured Original results for the stale-sprite `0xffff` action-point
    /// over-read. The original-game getter indexes beyond its delay table, so this value is
    /// allocator residue rather than reproducible simulation state. Parity
    /// replays may supply the observed wrapped signed 16-bit value, keyed by the
    /// proposer's and target's Original creation orders, for the current
    /// frame only.
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub(crate) original_impossible_action_done_deadlines: BTreeMap<(u32, u32), VecDeque<i16>>,
}

impl serde::Serialize for SimulationControl {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedSimulationControl::capture(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for SimulationControl {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(PersistedSimulationControl::deserialize(deserializer)?.into_runtime())
    }
}

/// Save-owned clock/configuration state. Admission capabilities and parity
/// callback observations belong to a live runtime, never a loaded mission.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PersistedSimulationControl {
    frame_counter: u32,
    simulation_gates: SimulationGateState,
    speed: f32,
    speed_int: u16,
    chorus_timer: u16,
    rng: u64,
    sim_config: SimConfig,
    mission_start_rng_seed: u64,
    mission_start_sim_config: SimConfig,
    fast_forward: bool,
    #[serde(default)]
    arrow_refresh_pending: bool,
    #[serde(default)]
    popup_scroll_last_display_frame: Option<u32>,
}

impl PersistedSimulationControl {
    pub(crate) fn capture(control: &SimulationControl) -> Result<Self, String> {
        let SimulationControl {
            frame_counter,
            simulation_gates,
            speed,
            speed_int,
            chorus_timer,
            rng,
            sim_config,
            mission_start_rng_seed,
            mission_start_sim_config,
            fast_forward,
            arrow_refresh_pending,
            popup_scroll_last_display_frame,
            ranked_simulation_policy: _,
            original_impossible_action_done_deadlines: _,
        } = control;
        sim_config.validate().map_err(|error| error.to_string())?;
        mission_start_sim_config
            .validate()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            frame_counter: *frame_counter,
            simulation_gates: simulation_gates.clone(),
            speed: *speed,
            speed_int: *speed_int,
            chorus_timer: *chorus_timer,
            rng: rng.persisted_seed()?,
            sim_config: *sim_config,
            mission_start_rng_seed: *mission_start_rng_seed,
            mission_start_sim_config: *mission_start_sim_config,
            fast_forward: *fast_forward,
            arrow_refresh_pending: *arrow_refresh_pending,
            popup_scroll_last_display_frame: *popup_scroll_last_display_frame,
        })
    }

    pub(crate) fn into_runtime(self) -> SimulationControl {
        SimulationControl {
            frame_counter: self.frame_counter,
            simulation_gates: self.simulation_gates,
            speed: self.speed,
            speed_int: self.speed_int,
            chorus_timer: self.chorus_timer,
            rng: SimulationRng::with_seed(self.rng),
            sim_config: self.sim_config,
            mission_start_rng_seed: self.mission_start_rng_seed,
            mission_start_sim_config: self.mission_start_sim_config,
            fast_forward: self.fast_forward,
            arrow_refresh_pending: self.arrow_refresh_pending,
            popup_scroll_last_display_frame: self.popup_scroll_last_display_frame,
            ranked_simulation_policy: None,
            original_impossible_action_done_deadlines: BTreeMap::new(),
        }
    }
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
            ranked_simulation_policy: None,
            arrow_refresh_pending: false,
            popup_scroll_last_display_frame: None,
            original_impossible_action_done_deadlines: BTreeMap::new(),
        }
    }

    pub(crate) fn simulation_context(&self) -> crate::sim_rng::SimulationContext {
        self.rng.context(self.sim_config)
    }

    pub(crate) fn install_ranked_simulation_policy(&mut self, policy: RankedSimulationPolicy) {
        policy
            .validate_config(self.sim_config)
            .unwrap_or_else(|error| {
                panic!("attempted to install incompatible ranked simulation policy: {error}")
            });
        assert!(
            self.ranked_simulation_policy.replace(policy).is_none(),
            "ranked simulation policy was installed more than once"
        );
    }

    pub(crate) const fn ranked_simulation_policy(&self) -> Option<RankedSimulationPolicy> {
        self.ranked_simulation_policy
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

    #[test]
    fn persistence_projection_preserves_clock_but_not_live_parity_observations() {
        let mut control = SimulationControl::new(0x1234, SimConfig::default());
        control.frame_counter = 81;
        control.speed = 2.5;
        control.popup_scroll_last_display_frame = Some(80);
        control
            .original_impossible_action_done_deadlines
            .insert((7, 9), VecDeque::from([12, -3]));
        let raw = control.clone();
        let persisted = PersistedSimulationControl::capture(&control).unwrap();
        let bytes = serde_json::to_vec(&persisted).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&control).unwrap());
        let restored = persisted.into_runtime();
        assert_eq!(restored.frame_counter, 81);
        assert_eq!(restored.speed, 2.5);
        assert_eq!(restored.popup_scroll_last_display_frame, Some(80));
        assert_eq!(restored.rng.seed(), 0x1234);
        assert!(
            restored
                .original_impossible_action_done_deadlines
                .is_empty()
        );
        assert_eq!(
            raw.original_impossible_action_done_deadlines[&(7, 9)],
            VecDeque::from([12, -3])
        );
        assert_eq!(
            robin_util::state_hash::compute(&raw),
            robin_util::state_hash::compute(&restored)
        );
        let decoded: SimulationControl = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            robin_util::state_hash::compute(&decoded),
            robin_util::state_hash::compute(&restored)
        );
    }
}
