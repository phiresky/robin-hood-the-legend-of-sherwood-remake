//! AI system — core types, state machine, stimulus processing.
//!
//! Defines the enums, flags, data structures, and base AI controller
//! that drive all NPC behavior. The actual behavior implementations live in
//! [`ai_enemy`](super::ai_enemy) (villain/soldier AI) and
//! [`ai_friendly`](super::ai_friendly) (civilian AI).

use std::sync::Arc;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::EntityId;
use crate::order::AiOrderIntent;

/// Score a building-door candidate exactly like Original
/// `GetNearestDoor`: narrow MaxNorm to UWORD, then apply both maluses with
/// wrapping 16-bit arithmetic.
pub(crate) fn legacy_nearest_door_distance(
    dx: f32,
    dy: f32,
    sector_changes: bool,
    layer_changes: bool,
) -> u16 {
    let mut distance = dx.abs().max(dy.abs()) as u16;
    if sector_changes {
        distance = distance.wrapping_add(500);
    }
    if layer_changes {
        distance = distance.wrapping_add(300);
    }
    distance
}

mod types;
pub(crate) use types::deserialize_optional_ai_handle;
pub use types::{
    AiEntityHandle, AiLockFlags, AiStateChangeSource, AlertFlags, CharlySeekerTarget, DoorHandle,
    DutyFlags, ElementHandle, EnterSwordfightRequest, GotoFlags, HALF_MAX_ATT_VALUE, HumanHandle,
    IntoOptionalAiHandle, MAX_ATT_VALUE, NpcHandle, ObjectHandle, QUARTER_MAX_ATT_VALUE,
    RemarkTargetFlags, SectorHandle, SpeechFlags, THREE_QUARTERS_MAX_ATT_VALUE,
};

mod macro_patrol;
pub use macro_patrol::{
    DetachedPatrolPathStatus, ForecastInput, ForecastedDestination, MacroOpcode, PATROL_SPEED_BASE,
    PATROL_SPEED_DIVISOR, PathHistoryEntry, PathId, PatrolPath, Position,
    PreparedForecastDestination, forecast_destination_for_ia, prepare_forecast_destination_for_ia,
};

mod model;
pub use model::{
    AMBUSH_BOX_HALF_SIZE, AiState, AlertContinuation, AlertLevel, AlertSoldiersFailureContinuation,
    AmbushPoint, Attitude, CombatInfo, CrossNpcAction, Curiosity, Decision, Detection,
    DoorCombatInfo, DoorSeekInfo, EmoticonType, ForbiddenRemark, Hint, LogLine, LogLineType,
    LookDirection, LookThereContinuation, Noise, NoiseOrigin, NoiseType, PanicRequest,
    PatrolAssignment, PointArchery, ProbabilityDistribution, Question, ReconnaissanceReport,
    Remark, ReportType, RepulsivePoint, ScreenRemark, ScriptSeekAreaRequest, SectorArchery,
    SeekPoint, SeekPointDirection, Stimulus, StimulusCategory, StimulusInfo, StimulusType,
    StolenObject, Substate, TargetType, ThinkResultContinuation, ViewCone,
    stimulus_to_ai_event_code,
};
pub(crate) use model::{
    QueuedSelfStimulus, SelfStimulusOrigin, cache_npc_villain_authorized_direct,
};

mod contexts;
pub use contexts::{
    AI_DOOR_RALLY_POINT_DISTANCE, AiContext, AiGlobalState, AiPerTickData, AntagonistInfo,
    DoorRallyPoint, FriendSwapCandidate, House, MyExitDoorInfo, PhalanxEnemySnapshot,
    PhalanxMemberThemList, ReconsiderSwordfightFriend, ReconsiderSwordfightObservationFighter,
    ReinforcementDoorInfo, SleepingEnemyInfo,
};

mod effects;
#[allow(unused_imports)]
pub(crate) use effects::{AiActorCoreEffects, AiActorPreemptionEffects};
pub use effects::{
    AiActorOutbox, AiDetectionOutbox, AiMusicOutbox, AiOutbox, AiOwnerWork, AiPatrolOutbox,
    AiRecoveryOutbox, AiReentrantOutbox, AiSpeechAttempt, AiStateChangeNotification,
    ArcheryReservationRelease, AttentiveModeEffect, GuardedPcEffect, InitStateSideEffects,
    ReservedShootingPoint,
};

mod controller;
pub(crate) use controller::PatrolCoordinateAction;
pub(crate) use controller::WillStopCaller;
pub(crate) use controller::consider_report_debug_matches;
pub use controller::{AiController, ConsiderationAccumulator};

#[cfg(test)]
mod tests;
