//! The single schedule for the mission loading bar.
//!
//! Every loading-screen target derives from [`LOADING_PHASE_WEIGHTS_MS`], so
//! the bar advances roughly in proportion to wall-clock time. Weights are
//! milliseconds measured on the single-threaded browser build (live
//! 23cc72ed0b2c, headless Chrome page refresh, Demo I auto-start) — the slowest
//! and most visible target. Re-measure from the `[loading]` and
//! `startup timing` info logs when a phase's cost changes.
//!
//! Status calls use two conventions of
//! [`LoadingScreenRenderer`](crate::loading_screen::LoadingScreenRenderer):
//! `set_status(text, phase.end())` starts an estimate-driven phase (the bar
//! snaps to the previous phase's end and ticks toward this phase's end), and
//! `set_counted_status(text, phase.at(fraction))` reports exact progress.

use serde::{Deserialize, Serialize};

use crate::shipping_mission::MissionLoadPhase;

/// Loading phases in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum LoadingPhase {
    /// Loading-screen construction and mission dependency planning.
    PrepareMissionData,
    /// Download and decompress the mission's shipping parts.
    MissionData,
    /// Decode sprite chunks (VQ grids and RLE-JXL atlases) at install.
    SpriteDecode,
    /// Speculative mission-audio warmup at the shipping boundary.
    MissionDataAudio,
    ProcessResources,
    /// Leaderboard API round trip for ranked admission.
    RankedAuthority,
    /// Interface archive setup, mission binaries, terrain decode start and
    /// the mission resource environment.
    InterfaceResources,
    SpriteBank,
    /// Mission scripts and engine construction.
    InitializeLevel,
    /// Background/minimap decode (inline only without a decode worker).
    MapDecode,
    SpriteVariants,
    MissionAudio,
    LevelDescriptors,
    HudFonts,
    /// Game renderer construction, map upload, interface decode join,
    /// frontend and HUD assembly. The loading screen hands its renderer to the
    /// mission when this starts, so the bar rests at this phase's start until
    /// the first mission frame replaces it.
    Finalizing,
}

/// Measured wall-clock weight of each phase, in execution order.
pub(super) const LOADING_PHASE_WEIGHTS_MS: [(LoadingPhase, u32); 15] = [
    (LoadingPhase::PrepareMissionData, 20),
    (LoadingPhase::MissionData, 1630),
    (LoadingPhase::SpriteDecode, 6000),
    (LoadingPhase::MissionDataAudio, 20),
    (LoadingPhase::ProcessResources, 60),
    (LoadingPhase::RankedAuthority, 370),
    (LoadingPhase::InterfaceResources, 200),
    (LoadingPhase::SpriteBank, 100),
    (LoadingPhase::InitializeLevel, 350),
    (LoadingPhase::MapDecode, 50),
    (LoadingPhase::SpriteVariants, 50),
    (LoadingPhase::MissionAudio, 30),
    (LoadingPhase::LevelDescriptors, 30),
    (LoadingPhase::HudFonts, 20),
    (LoadingPhase::Finalizing, 930),
];

/// Worker-pool streaming builds decode activation-critical sprite chunks
/// while parts download, so their `Data` progress also covers this share of
/// [`LoadingPhase::SpriteDecode`]; the install-time remainder covers the rest.
// TODO: measure on a cross-origin-isolated threaded deploy and retune.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
const STREAMING_DATA_SPRITE_DECODE_SHARE: f32 = 0.75;

impl LoadingPhase {
    fn weight_bounds(self) -> (u32, u32) {
        let mut before = 0;
        for (phase, weight) in LOADING_PHASE_WEIGHTS_MS {
            if phase == self {
                return (before, before + weight);
            }
            before += weight;
        }
        panic!("loading phase {self:?} is missing from LOADING_PHASE_WEIGHTS_MS");
    }

    fn total_weight() -> f32 {
        LOADING_PHASE_WEIGHTS_MS
            .iter()
            .map(|(_, weight)| weight)
            .sum::<u32>() as f32
    }

    /// Bar position when this phase starts, in `0.0..=1.0`.
    pub(super) fn start(self) -> f32 {
        self.weight_bounds().0 as f32 / Self::total_weight()
    }

    /// Bar position when this phase ends, in `0.0..=1.0`.
    pub(super) fn end(self) -> f32 {
        self.weight_bounds().1 as f32 / Self::total_weight()
    }

    /// Bar position `fraction` of the way through this phase.
    pub(super) fn at(self, fraction: f32) -> f32 {
        lerp(self.start(), self.end(), fraction)
    }
}

fn lerp(start: f32, end: f32, fraction: f32) -> f32 {
    assert!(
        fraction.is_finite(),
        "loading progress fraction is not finite"
    );
    start + (end - start) * fraction.clamp(0.0, 1.0)
}

/// Bar position for asynchronous shipping-data progress.
pub(super) fn shipping_progress_target(phase: MissionLoadPhase, fraction: f32) -> f32 {
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    {
        let split = LoadingPhase::SpriteDecode.at(STREAMING_DATA_SPRITE_DECODE_SHARE);
        match phase {
            MissionLoadPhase::Data => lerp(LoadingPhase::MissionData.start(), split, fraction),
            MissionLoadPhase::Sprites => lerp(split, LoadingPhase::SpriteDecode.end(), fraction),
            MissionLoadPhase::Audio => LoadingPhase::MissionDataAudio.at(fraction),
        }
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "wasm-threads")))]
    match phase {
        MissionLoadPhase::Data => LoadingPhase::MissionData.at(fraction),
        MissionLoadPhase::Sprites => LoadingPhase::SpriteDecode.at(fraction),
        MissionLoadPhase::Audio => LoadingPhase::MissionDataAudio.at(fraction),
    }
}

#[cfg(test)]
mod tests {
    use super::{LOADING_PHASE_WEIGHTS_MS, LoadingPhase, shipping_progress_target};
    use crate::loading_screen::LoadingScreen;
    use crate::shipping_mission::MissionLoadPhase;

    #[test]
    fn phases_tile_the_whole_bar_in_order() {
        let mut previous_end = 0.0;
        for (phase, weight) in LOADING_PHASE_WEIGHTS_MS {
            assert!(weight > 0, "{phase:?} has no weight");
            assert_eq!(phase.start(), previous_end, "{phase:?} leaves a gap");
            assert!(phase.end() > phase.start(), "{phase:?} is empty");
            previous_end = phase.end();
        }
        assert_eq!(LOADING_PHASE_WEIGHTS_MS[0].0.start(), 0.0);
        assert_eq!(previous_end, 1.0);
    }

    #[test]
    fn every_phase_appears_exactly_once() {
        for (index, (phase, _)) in LOADING_PHASE_WEIGHTS_MS.iter().enumerate() {
            assert!(
                LOADING_PHASE_WEIGHTS_MS[index + 1..]
                    .iter()
                    .all(|(other, _)| other != phase),
                "{phase:?} is listed twice"
            );
        }
    }

    #[test]
    fn positions_within_a_phase_are_monotonic_and_clamped() {
        let phase = LoadingPhase::SpriteDecode;
        let mut previous = phase.start();
        for step in 0..=100 {
            let value = phase.at(step as f32 / 100.0);
            assert!(value >= previous);
            previous = value;
        }
        assert_eq!(phase.at(-1.0), phase.start());
        assert_eq!(phase.at(2.0), phase.end());
    }

    #[test]
    #[should_panic(expected = "not finite")]
    fn non_finite_fraction_is_rejected() {
        LoadingPhase::MissionData.at(f32::NAN);
    }

    #[test]
    fn sprite_decode_dominates_as_measured() {
        let span = LoadingPhase::SpriteDecode.end() - LoadingPhase::SpriteDecode.start();
        assert!(span > 0.5, "sprite decode covers {span} of the bar");
    }

    /// Replays the interactive status order through the logical loading
    /// state: no update may move the bar backwards (which would log a
    /// warning and freeze it).
    #[test]
    fn interactive_status_sequence_never_moves_backwards() {
        let mut targets = vec![LoadingPhase::PrepareMissionData.end()];
        for phase in [
            MissionLoadPhase::Data,
            MissionLoadPhase::Sprites,
            MissionLoadPhase::Audio,
        ] {
            for step in 0..=10 {
                targets.push(shipping_progress_target(phase, step as f32 / 10.0));
            }
        }
        for phase in [
            LoadingPhase::ProcessResources,
            LoadingPhase::RankedAuthority,
            LoadingPhase::InterfaceResources,
            LoadingPhase::SpriteBank,
            LoadingPhase::InitializeLevel,
            LoadingPhase::MapDecode,
            LoadingPhase::SpriteVariants,
            LoadingPhase::MissionAudio,
            LoadingPhase::LevelDescriptors,
            LoadingPhase::HudFonts,
            LoadingPhase::Finalizing,
        ] {
            // Estimate phases display their start and tick toward their end.
            targets.push(phase.start());
            targets.push(phase.end());
        }
        let mut state = LoadingScreen::default();
        state.initialize(640, 480, 22.0);
        for target in targets {
            let level = target * state.max_level;
            assert!(
                level >= state.current_level,
                "target {target} moves the bar backwards from {}",
                state.progress()
            );
            state.update_level(level);
        }
        assert_eq!(state.progress(), 1.0);
    }
}
