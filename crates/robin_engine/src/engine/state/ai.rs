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
    /// Active synchronous decision frames, shared by all actors.
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub(crate) think_call_stack: Vec<crate::element::EntityId>,
    pub(crate) global: AiGlobalState,
    pub(crate) standard_view_polygon_radius: u16,
    /// Authoritative within-frame memo state stored on the ground singleton
    /// and projection obstacles in Original.
    pub(crate) view_radius_cache: crate::ai_vision::ViewRadiusCache,
}

impl AiRuntime {
    pub(crate) fn persisted_clone(&self) -> Self {
        let value = self;
        use crate::ai::persisted::PersistedProjection;
        let AiRuntime {
            think_call_stack: _,
            global: _,
            standard_view_polygon_radius: _,
            view_radius_cache: _,
        } = value;
        Self {
            think_call_stack: Vec::new(),
            global: value.global.persisted_clone(),
            standard_view_polygon_radius: value.standard_view_polygon_radius,
            view_radius_cache: value.view_radius_cache.clone(),
        }
    }
}

impl AiRuntime {
    pub(crate) fn new() -> Self {
        Self {
            think_call_stack: Vec::new(),
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
