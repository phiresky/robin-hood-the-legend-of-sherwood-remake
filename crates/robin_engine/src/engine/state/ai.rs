use serde::{Deserialize, Serialize};

use crate::ai::AiGlobalState;

/// Deterministic global AI state and mission-configured vision defaults.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct AiRuntime {
    pub(crate) global: AiGlobalState,
    pub(crate) standard_view_polygon_radius: u16,
    /// Authoritative within-frame memo state stored on the ground singleton
    /// and projection obstacles in Original.
    pub(crate) view_radius_cache: crate::ai_vision::ViewRadiusCache,
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedAiRuntime {
    global: crate::ai::persisted::PersistedAiGlobalState,

    standard_view_polygon_radius: u16,

    view_radius_cache: crate::ai_vision::ViewRadiusCache,
}

impl PersistedAiRuntime {
    pub(crate) fn capture(value: &AiRuntime) -> Self {
        let AiRuntime {
            global: _,
            standard_view_polygon_radius: _,
            view_radius_cache: _,
        } = value;
        Self {
            global: crate::ai::persisted::PersistedAiGlobalState::capture(&value.global),
            standard_view_polygon_radius: value.standard_view_polygon_radius.clone(),
            view_radius_cache: value.view_radius_cache.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> AiRuntime {
        AiRuntime {
            global: self.global.into_runtime(),
            standard_view_polygon_radius: self.standard_view_polygon_radius,
            view_radius_cache: self.view_radius_cache,
        }
    }
}

impl AiRuntime {
    pub(crate) fn new() -> Self {
        Self {
            global: AiGlobalState::default(),
            standard_view_polygon_radius: 0,
            view_radius_cache: crate::ai_vision::ViewRadiusCache::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ai_runtime_has_no_mission_specific_vision_state() {
        let ai = AiRuntime::new();

        assert_eq!(ai.standard_view_polygon_radius, 0);
        assert!(ai.view_radius_cache.is_empty());
        assert!(ai.global.seek_points.is_empty());
        assert!(ai.global.ambush_points.is_empty());
    }
}
