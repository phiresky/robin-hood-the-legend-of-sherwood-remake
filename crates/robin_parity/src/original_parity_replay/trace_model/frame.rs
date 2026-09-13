//! Frame record layout and record-type marker.
use super::command::TraceCommand;
use super::element::{TraceElement, TraceVisibilityQuery};
use super::events::{
    TraceAiForecastEvent, TraceAlertFormationEvent, TraceFlightStep, TraceGoToAuthorizationEvent,
    TraceMovementStep, TracePathEvent, TracePopupEvent, TraceResolvedExclamation,
    TraceRouteConstructionEvent, TraceSequenceLifecycleEvent, TraceStrikeProposalEvent,
    TraceTargetLifecycleEvent,
};
use super::motion::TraceMotionLineChange;
use super::rng::TraceRngBatch;
use super::scalar::TraceEntityId;
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum TraceDirectorCompletion {
    CameraGoto,
    ZoomLevel,
}

impl From<TraceDirectorCompletion> for robin_engine::engine::DirectorCompletion {
    fn from(value: TraceDirectorCompletion) -> Self {
        match value {
            TraceDirectorCompletion::CameraGoto => Self::CameraGoto,
            TraceDirectorCompletion::ZoomLevel => Self::ZoomLevel,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceRecordMarker {
    #[serde(rename = "type")]
    pub(crate) record_type: Option<String>,
}

/// Frame layout embedded in version-68 native records.
///
/// ON-DISK FORMAT INVARIANT: any bitcode-shape change requires a native
/// version bump plus a frozen compatibility decoder for this layout.
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceFrame {
    #[serde(rename = "type")]
    pub(crate) record_type: String,
    pub(crate) frame_before: u64,
    pub(crate) frame_after: u64,
    pub(crate) game_code: i32,
    pub(crate) simulation_body_ran: bool,
    pub(crate) commands: Vec<TraceCommand>,
    pub(crate) director_completions: Vec<TraceDirectorCompletion>,
    pub(crate) selected_pcs: Vec<TraceEntityId>,
    pub(crate) elements: Vec<TraceElement>,
    pub(crate) visibility_queries: Vec<TraceVisibilityQuery>,
    pub(crate) rng_draws: TraceRngBatch,
    pub(crate) motion_line_changes: Vec<TraceMotionLineChange>,
    pub(crate) path_events: Vec<TracePathEvent>,
    /// Retained and printed on divergence; Rust has no
    /// side-effect-free route-construction event capture yet.
    /// TODO(parity-schema): compare once the sequence builders publish the
    /// same source/goal and ordered gate list.
    pub(crate) route_construction_events: Vec<TraceRouteConstructionEvent>,
    pub(crate) popup_events: Vec<TracePopupEvent>,
    pub(crate) ai_forecast_events: Vec<TraceAiForecastEvent>,
    pub(crate) alert_formation_events: Vec<TraceAlertFormationEvent>,
    pub(crate) goto_authorization_events: Vec<TraceGoToAuthorizationEvent>,
    pub(crate) strike_proposal_events: Vec<TraceStrikeProposalEvent>,
    pub(crate) sequence_lifecycle_events: Vec<TraceSequenceLifecycleEvent>,
    pub(crate) target_lifecycle_events: Vec<TraceTargetLifecycleEvent>,
    pub(crate) resolved_exclamations: Vec<TraceResolvedExclamation>,
    /// Optional diagnostics in early schema-16 recordings. They are retained
    /// whenever present and default only at the JSON-to-native compatibility
    /// boundary; logical state comparison never invents recorded operands.
    #[serde(default)]
    pub(crate) movement_steps: Vec<TraceMovementStep>,
    #[serde(default)]
    pub(crate) flight_steps: Vec<TraceFlightStep>,
}
