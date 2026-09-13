//! Current trace DTOs and scalar representations; wire field order and types are unchanged.
//!
//! The wire types are grouped into submodules; this root declares the surface
//! the rest of the parity replay may use.
mod campaign;
mod command;
mod element;
mod events;
mod frame;
mod header;
mod json;
mod motion;
mod rng;
mod scalar;

pub(super) use campaign::TraceCampaign;
pub(super) use command::{TraceAction, TraceCommand};
pub(super) use element::{
    TraceActor, TraceElement, TraceHuman, TraceJumpLine, TracePassDoor, TraceVisibilityQuery,
};
pub(super) use events::{
    TraceFlightStep, TraceMovementStep, TracePathEvent, TraceRouteConstructionEvent,
    TraceSequenceLifecycleEvent,
};
pub(super) use frame::{TraceFrame, TraceRecordMarker};
pub(super) use header::{
    LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW, OLDEST_SUPPORTED_TRACE_SCHEMA, TRACE_SCHEMA_VERSION,
    TraceHeader, TraceInitialNpcTransient, TraceStartState,
};
pub(super) use json::{TraceJsonTree, TraceJsonValue};
pub(super) use motion::{TraceMotionGrid, TraceMotionLine, TraceMotionLineChange};
pub(super) use rng::{TraceRngBatch, TraceRngDomain, TraceRngOnly, TraceRngPrefix};
pub(super) use scalar::{TraceEntityId, TraceEntityKind, TraceFloat, TracePoint};

// Nested record types only the unit tests construct or decode directly.
#[cfg(test)]
pub(super) use element::{TraceAi, TraceSequenceElement, TraceSequenceMovement};
#[cfg(test)]
pub(super) use events::{
    TraceAlertEligibility, TraceRouteGate, TraceTargetLifecycleEvent, TraceTargetLifecyclePayload,
};
#[cfg(test)]
pub(super) use header::{
    TraceDifficulty, TraceInitialSave, TraceSaveSourceProfile, TraceSimConfig,
};
#[cfg(test)]
pub(super) use json::missing_legacy_trace_json_value;
#[cfg(test)]
pub(super) use scalar::TracePoint3;
