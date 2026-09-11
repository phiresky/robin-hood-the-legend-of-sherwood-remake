use serde::{Deserialize, Serialize};

use crate::{
    coordinates::{MapPoint, MapSize},
    engine::{CameraState, SideEffects},
    markers::GroundMark,
    sound::SoundSimState,
    titbit::TitbitManager,
};

/// Deterministic feedback production and shared director presentation state.
///
/// Local viewport/UI animation remains host-owned; every field here is part of
/// rollback and deterministic hashing.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct FeedbackRuntime {
    pub(crate) sound_sim: SoundSimState,
    pub(crate) ground_mark: GroundMark,
    pub(crate) titbit_manager: TitbitManager,
    pub(crate) cutscene_camera: CameraState,
    pub(crate) pending_side_effects: SideEffects,
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedFeedbackRuntime {
    sound_sim: SoundSimState,

    ground_mark: GroundMark,

    titbit_manager: TitbitManager,

    cutscene_camera: CameraState,

    pending_side_effects: crate::engine::PersistedSideEffects,
}

impl PersistedFeedbackRuntime {
    pub(crate) fn capture(value: &FeedbackRuntime) -> Self {
        let FeedbackRuntime {
            sound_sim: _,
            ground_mark: _,
            titbit_manager: _,
            cutscene_camera: _,
            pending_side_effects: _,
        } = value;
        Self {
            sound_sim: value.sound_sim.clone(),
            ground_mark: value.ground_mark.clone(),
            titbit_manager: value.titbit_manager.clone(),
            cutscene_camera: value.cutscene_camera.clone(),
            pending_side_effects: crate::engine::PersistedSideEffects::capture(
                &value.pending_side_effects,
            ),
        }
    }

    pub(crate) fn into_runtime(self) -> FeedbackRuntime {
        FeedbackRuntime {
            sound_sim: self.sound_sim,
            ground_mark: self.ground_mark,
            titbit_manager: self.titbit_manager,
            cutscene_camera: self.cutscene_camera,
            pending_side_effects: self.pending_side_effects.into_runtime(),
        }
    }
}

impl FeedbackRuntime {
    pub(crate) fn new() -> Self {
        Self {
            sound_sim: SoundSimState::default(),
            ground_mark: GroundMark::default(),
            titbit_manager: TitbitManager::new(),
            cutscene_camera: CameraState {
                level_size: MapSize::ZERO,
                zoom_factor: 1.0,
                desired_zoom_factor: 1.0,
                camera_slide: MapPoint::new(-1.0, -1.0),
                ..Default::default()
            },
            pending_side_effects: SideEffects::default(),
        }
    }

    pub(crate) fn drain_side_effects(&mut self) -> SideEffects {
        std::mem::take(&mut self.pending_side_effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_feedback_has_canonical_director_camera_and_empty_outputs() {
        let feedback = FeedbackRuntime::new();

        assert_eq!(feedback.cutscene_camera.level_size, MapSize::ZERO);
        assert_eq!(feedback.cutscene_camera.zoom_factor, 1.0);
        assert_eq!(feedback.cutscene_camera.desired_zoom_factor, 1.0);
        assert_eq!(
            feedback.cutscene_camera.camera_slide,
            MapPoint::new(-1.0, -1.0)
        );
        assert_eq!(feedback.pending_side_effects.code, Default::default());
        assert!(feedback.pending_side_effects.sounds.is_empty());
        assert!(feedback.pending_side_effects.overlay.is_none());
    }
}
